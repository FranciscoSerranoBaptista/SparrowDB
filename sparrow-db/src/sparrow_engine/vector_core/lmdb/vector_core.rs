use super::{binary_heap::BinaryHeap, hnsw::HNSW, utils::{Candidate, HeapOps, VectorFilter}};
use crate::{
    debug_println,
    sparrow_engine::{
        types::VectorError,
        vector_core::{
            vector::HVector,
            vector_without_data::VectorWithoutData,
        },
    },
    utils::{id::uuid_str, properties::ImmutablePropertiesMap},
};
use heed3::{
    Database, Env, RoTxn, RwTxn,
    byteorder::BE,
    types::{Bytes, U128, Unit},
};
use rand::prelude::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const DB_VECTORS: &str = "vectors"; // for vector data (v:)
const DB_VECTOR_DATA: &str = "vector_data"; // for vector data (v:)
const DB_HNSW_EDGES: &str = "hnsw_out_nodes"; // for hnsw out node data
const VECTOR_PREFIX: &[u8] = b"v:";
pub const ENTRY_POINT_KEY: &[u8] = b"entry_point";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HNSWConfig {
    pub m: usize,             // max num of bi-directional links per element
    pub m_max_0: usize,       // max num of links for lower layers
    pub ef_construct: usize,  // size of the dynamic candidate list for construction
    pub m_l: f64,             // level generation factor
    pub ef: usize,            // search param, num of cands to search
    pub min_neighbors: usize, // for get_neighbors, always 512
}

impl HNSWConfig {
    /// Constructor for the configs of the HNSW vector similarity search algorithm
    /// - m (5 <= m <= 48): max num of bi-directional links per element
    /// - m_max_0 (2 * m): max num of links for level 0 (level that stores all vecs)
    /// - ef_construct (40 <= ef_construct <= 512): size of the dynamic candidate list
    ///   for construction
    /// - m_l (ln(1/m)): level generation factor (multiplied by a random number)
    /// - ef (10 <= ef <= 512): num of candidates to search
    pub fn new(m: Option<usize>, ef_construct: Option<usize>, ef: Option<usize>) -> Self {
        let m = m.unwrap_or(16).clamp(5, 48);
        let ef_construct = ef_construct.unwrap_or(128).clamp(40, 512);
        let ef = ef.unwrap_or(768).clamp(10, 512);

        Self {
            m,
            m_max_0: 2 * m,
            ef_construct,
            m_l: 1.0 / (m as f64).ln(),
            ef,
            min_neighbors: 512,
        }
    }
}

pub struct VectorStats {
    pub total: u64,
    pub active: u64,
    pub soft_deleted: u64,
    pub hnsw_edges: u64,
    pub entry_point_present: bool,
}

pub struct VectorCore {
    pub vectors_db: Database<Bytes, Bytes>,
    pub vector_properties_db: Database<U128<BE>, Bytes>,
    pub edges_db: Database<Bytes, Unit>,
    pub config: HNSWConfig,
}

impl VectorCore {
    pub fn new(env: &Env, txn: &mut RwTxn, config: HNSWConfig) -> Result<Self, VectorError> {
        let vectors_db = env.create_database(txn, Some(DB_VECTORS))?;
        let vector_properties_db = env
            .database_options()
            .types::<U128<BE>, Bytes>()
            .name(DB_VECTOR_DATA)
            .create(txn)?;
        let edges_db = env.create_database(txn, Some(DB_HNSW_EDGES))?;

        Ok(Self {
            vectors_db,
            vector_properties_db,
            edges_db,
            config,
        })
    }

    /// Vector key: [v, id, ]
    #[inline(always)]
    pub fn vector_key(id: u128, level: usize) -> Vec<u8> {
        [VECTOR_PREFIX, &id.to_be_bytes(), &level.to_be_bytes()].concat()
    }

    #[inline(always)]
    pub fn out_edges_key(source_id: u128, level: usize, sink_id: Option<u128>) -> Vec<u8> {
        match sink_id {
            Some(sink_id) => [
                source_id.to_be_bytes().as_slice(),
                level.to_be_bytes().as_slice(),
                sink_id.to_be_bytes().as_slice(),
            ]
            .concat()
            .to_vec(),
            None => [
                source_id.to_be_bytes().as_slice(),
                level.to_be_bytes().as_slice(),
            ]
            .concat()
            .to_vec(),
        }
    }

    #[inline]
    fn get_new_level(&self) -> usize {
        let mut rng = rand::rng();
        let r: f64 = rng.random::<f64>();
        (-r.ln() * self.config.m_l).floor() as usize
    }

    #[inline]
    fn get_entry_point<'db: 'arena, 'arena: 'txn, 'txn>(
        &self,
        txn: &'txn RoTxn<'db>,
        label: &'arena str,
        arena: &'arena bumpalo::Bump,
    ) -> Result<HVector<'arena>, VectorError> {
        let ep_id = self.vectors_db.get(txn, ENTRY_POINT_KEY)?;
        if let Some(ep_id) = ep_id {
            let mut arr = [0u8; 16];
            let len = std::cmp::min(ep_id.len(), 16);
            arr[..len].copy_from_slice(&ep_id[..len]);
            self.get_raw_vector_data(txn, u128::from_be_bytes(arr), label, arena)
        } else {
            Err(VectorError::EntryPointNotFound)
        }
    }

    #[inline]
    fn set_entry_point(&self, txn: &mut RwTxn, entry: &HVector) -> Result<(), VectorError> {
        self.vectors_db
            .put(txn, ENTRY_POINT_KEY, &entry.id.to_be_bytes())
            .map_err(VectorError::from)?;
        Ok(())
    }

    #[inline(always)]
    pub fn put_vector<'arena>(
        &self,
        txn: &mut RwTxn,
        vector: &HVector<'arena>,
    ) -> Result<(), VectorError> {
        self.vectors_db
            .put(
                txn,
                &Self::vector_key(vector.id, vector.level),
                vector.vector_data_to_bytes()?,
            )
            .map_err(VectorError::from)?;
        self.vector_properties_db
            .put(txn, &vector.id, bincode::serialize(&vector)?.as_ref())?;
        Ok(())
    }

    #[inline(always)]
    fn get_neighbors<'db: 'arena, 'arena: 'txn, 'txn, F>(
        &self,
        txn: &'txn RoTxn<'db>,
        label: &'arena str,
        id: u128,
        level: usize,
        filter: Option<&[F]>,
        arena: &'arena bumpalo::Bump,
    ) -> Result<bumpalo::collections::Vec<'arena, HVector<'arena>>, VectorError>
    where
        F: Fn(&HVector<'arena>, &RoTxn<'db>) -> bool,
    {
        let out_key = Self::out_edges_key(id, level, None);
        let mut neighbors = bumpalo::collections::Vec::with_capacity_in(
            self.config.m_max_0.min(self.config.min_neighbors),
            arena,
        );

        let iter = self
            .edges_db
            .lazily_decode_data()
            .prefix_iter(txn, &out_key)?;

        let prefix_len = out_key.len();

        for result in iter {
            let (key, _) = result?;

            let mut arr = [0u8; 16];
            arr[..16].copy_from_slice(&key[prefix_len..(prefix_len + 16)]);
            let neighbor_id = u128::from_be_bytes(arr);

            if neighbor_id == id {
                continue;
            }
            let vector = self.get_raw_vector_data(txn, neighbor_id, label, arena)?;

            let passes_filters = match filter {
                Some(filter_slice) => filter_slice.iter().all(|f| f(&vector, txn)),
                None => true,
            };

            if passes_filters {
                neighbors.push(vector);
            }
        }
        neighbors.shrink_to_fit();

        Ok(neighbors)
    }

    #[inline(always)]
    fn set_neighbours<'db: 'arena, 'arena: 'txn, 'txn, 's>(
        &'db self,
        txn: &'txn mut RwTxn<'db>,
        id: u128,
        neighbors: &BinaryHeap<'arena, HVector<'arena>>,
        level: usize,
        arena: &'arena bumpalo::Bump,
    ) -> Result<(), VectorError> {
        let prefix = Self::out_edges_key(id, level, None);

        let mut keys_to_delete: HashSet<Vec<u8>> = self
            .edges_db
            .prefix_iter(txn, prefix.as_ref())?
            .filter_map(|result| result.ok().map(|(key, _)| key.to_vec()))
            .collect();

        let limit = if level == 0 {
            self.config.m_max_0
        } else {
            self.config.m
        };

        neighbors
            .iter()
            .try_for_each(|neighbor| -> Result<(), VectorError> {
                let neighbor_id = neighbor.id;
                if neighbor_id == id {
                    return Ok(());
                }

                let out_key = Self::out_edges_key(id, level, Some(neighbor_id));
                keys_to_delete.remove(&out_key);
                self.edges_db.put(txn, &out_key, &())?;

                // Back-link: neighbor_id → id.  Stored under neighbor_id's prefix so it is
                // never in keys_to_delete (which only contains id's own outgoing edges).
                let in_key = Self::out_edges_key(neighbor_id, level, Some(id));
                self.edges_db.put(txn, &in_key, &())?;

                self.prune_if_over_degree(txn, neighbor_id, neighbor, level, limit, arena)?;

                Ok(())
            })?;

        for key in keys_to_delete {
            self.edges_db.delete(txn, &key)?;
        }

        Ok(())
    }

    fn prune_if_over_degree<'db: 'arena, 'arena: 'txn, 'txn>(
        &'db self,
        txn: &'txn mut RwTxn<'db>,
        node_id: u128,
        node_vec: &HVector<'_>,
        level: usize,
        limit: usize,
        arena: &'arena bumpalo::Bump,
    ) -> Result<(), VectorError> {
        let edge_prefix = Self::out_edges_key(node_id, level, None);

        // Collect all current neighbor IDs for this node at this level (two-phase: read then write)
        let neighbor_ids: Vec<u128> = self
            .edges_db
            .prefix_iter(txn, edge_prefix.as_ref())?
            .filter_map(|r| r.ok())
            .filter_map(|(key, _)| {
                if key.len() == 40 {
                    let mut arr = [0u8; 16];
                    arr.copy_from_slice(&key[24..40]);
                    Some(u128::from_be_bytes(arr))
                } else {
                    debug_assert!(false, "malformed HNSW edge key: expected 40 bytes, got {}", key.len());
                    None
                }
            })
            .collect();

        if neighbor_ids.len() <= limit {
            return Ok(());
        }

        // Compute distance from each neighbor to node_vec, then keep the closest `limit`
        let mut scored: Vec<(u128, f64)> = neighbor_ids
            .iter()
            .filter_map(|&nid| {
                let v = self.get_raw_vector_data(txn, nid, node_vec.label, arena).ok()?;
                let dist = v.distance_to(node_vec).ok()?;
                Some((nid, dist))
            })
            .collect();

        // Sort ascending by distance — closest neighbors are kept
        scored.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        for (nid, _) in &scored[limit..] {
            let mut fwd = [0u8; 40];
            fwd[..16].copy_from_slice(&node_id.to_be_bytes());
            fwd[16..24].copy_from_slice(&level.to_be_bytes());
            fwd[24..40].copy_from_slice(&nid.to_be_bytes());

            let mut rev = [0u8; 40];
            rev[..16].copy_from_slice(&nid.to_be_bytes());
            rev[16..24].copy_from_slice(&level.to_be_bytes());
            rev[24..40].copy_from_slice(&node_id.to_be_bytes());

            let _ = self.edges_db.delete(txn, &fwd);
            let _ = self.edges_db.delete(txn, &rev);
        }

        Ok(())
    }

    fn select_neighbors<'db: 'arena, 'arena: 'txn, 'txn, 's, F>(
        &'db self,
        txn: &'txn RoTxn<'db>,
        label: &'arena str,
        query: &'s HVector<'arena>,
        mut cands: BinaryHeap<'arena, HVector<'arena>>,
        level: usize,
        should_extend: bool,
        filter: Option<&[F]>,
        arena: &'arena bumpalo::Bump,
    ) -> Result<BinaryHeap<'arena, HVector<'arena>>, VectorError>
    where
        F: Fn(&HVector<'arena>, &RoTxn<'db>) -> bool,
    {
        let m = self.config.m;

        if !should_extend {
            return Ok(cands.take_inord(m));
        }

        let mut visited: HashSet<u128> = HashSet::new();
        let mut result = BinaryHeap::with_capacity(arena, m * cands.len());
        for candidate in cands.iter() {
            for mut neighbor in
                self.get_neighbors(txn, label, candidate.id, level, filter, arena)?
            {
                if !visited.insert(neighbor.id) {
                    continue;
                }

                neighbor.set_distance(neighbor.distance_to(query)?);

                /*
                let passes_filters = match filter {
                    Some(filter_slice) => filter_slice.iter().all(|f| f(&neighbor, txn)),
                    None => true,
                };

                if passes_filters {
                    result.push(neighbor);
                }
                */

                if filter.is_none() || filter.unwrap().iter().all(|f| f(&neighbor, txn)) {
                    result.push(neighbor);
                }
            }
        }

        result.extend(cands);
        Ok(result.take_inord(m))
    }

    fn search_level<'db: 'arena, 'arena: 'txn, 'txn, 'q, F>(
        &self,
        txn: &'txn RoTxn<'db>,
        label: &'arena str,
        query: &'q HVector<'arena>,
        entry_point: &'q mut HVector<'arena>,
        ef: usize,
        level: usize,
        filter: Option<&[F]>,
        arena: &'arena bumpalo::Bump,
    ) -> Result<BinaryHeap<'arena, HVector<'arena>>, VectorError>
    where
        F: Fn(&HVector<'arena>, &RoTxn<'db>) -> bool,
    {
        let mut visited: HashSet<u128> = HashSet::new();
        let mut candidates: BinaryHeap<'arena, Candidate> =
            BinaryHeap::with_capacity(arena, self.config.ef_construct);
        let mut results: BinaryHeap<'arena, HVector<'arena>> = BinaryHeap::new(arena);

        entry_point.set_distance(entry_point.distance_to(query)?);
        candidates.push(Candidate {
            id: entry_point.id,
            distance: entry_point.get_distance(),
        });
        results.push(*entry_point);
        visited.insert(entry_point.id);

        while let Some(curr_cand) = candidates.pop() {
            if results.len() >= ef
                && results
                    .get_max()
                    .is_none_or(|f| curr_cand.distance > f.get_distance())
            {
                break;
            }

            let max_distance = if results.len() >= ef {
                results.get_max().map(|f| f.get_distance())
            } else {
                None
            };

            self.get_neighbors(txn, label, curr_cand.id, level, filter, arena)?
                .into_iter()
                .filter(|neighbor| visited.insert(neighbor.id))
                .filter_map(|mut neighbor| {
                    let distance = neighbor.distance_to(query).ok()?;

                    if max_distance.is_none_or(|max| distance < max) {
                        neighbor.set_distance(distance);
                        Some((neighbor, distance))
                    } else {
                        None
                    }
                })
                .for_each(|(neighbor, distance)| {
                    candidates.push(Candidate {
                        id: neighbor.id,
                        distance,
                    });

                    results.push(neighbor);

                    if results.len() > ef {
                        results = results.take_inord(ef);
                    }
                });
        }
        Ok(results)
    }

    pub fn num_inserted_vectors(&self, txn: &RoTxn) -> Result<u64, VectorError> {
        Ok(self.vectors_db.len(txn)?)
    }

    pub fn stats<'db>(&self, txn: &RoTxn<'db>) -> Result<VectorStats, VectorError> {
        let mut total: u64 = 0;
        let mut soft_deleted: u64 = 0;

        let arena = bumpalo::Bump::new();
        let iter = self.vector_properties_db.iter(txn)?;
        for result in iter {
            let (id, bytes) = result?;
            let props = VectorWithoutData::from_bincode_bytes(&arena, bytes, id)
                .map_err(|e| VectorError::VectorCoreError(e.to_string()))?;
            total += 1;
            if props.deleted {
                soft_deleted += 1;
            }
        }

        let hnsw_edges = self.edges_db.len(txn)? as u64;
        let entry_point_present = self.vectors_db.get(txn, ENTRY_POINT_KEY)?.is_some();

        Ok(VectorStats {
            total,
            active: total.saturating_sub(soft_deleted),
            soft_deleted,
            hnsw_edges,
            entry_point_present,
        })
    }

    #[inline]
    pub fn get_vector_properties<'db: 'arena, 'arena: 'txn, 'txn>(
        &self,
        txn: &'txn RoTxn<'db>,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<Option<VectorWithoutData<'arena>>, VectorError> {
        let vector: Option<VectorWithoutData<'arena>> =
            match self.vector_properties_db.get(txn, &id)? {
                Some(bytes) => Some(VectorWithoutData::from_bincode_bytes(arena, bytes, id)?),
                None => None,
            };

        if let Some(vector) = vector
            && vector.deleted
        {
            return Err(VectorError::VectorDeleted);
        }

        Ok(vector)
    }

    #[inline(always)]
    pub fn get_full_vector<'arena>(
        &self,
        txn: &RoTxn,
        id: u128,
        arena: &'arena bumpalo::Bump,
    ) -> Result<HVector<'arena>, VectorError> {
        let vector_data_bytes = self
            .vectors_db
            .get(txn, &Self::vector_key(id, 0))?
            .ok_or(VectorError::VectorNotFound(uuid_str(id, arena).to_string()))?;

        let properties_bytes = self.vector_properties_db.get(txn, &id)?;

        let vector = HVector::from_bincode_bytes(arena, properties_bytes, vector_data_bytes, id)?;
        if vector.deleted {
            return Err(VectorError::VectorDeleted);
        }
        Ok(vector)
    }

    #[inline(always)]
    pub fn get_raw_vector_data<'db: 'arena, 'arena: 'txn, 'txn>(
        &self,
        txn: &'txn RoTxn<'db>,
        id: u128,
        label: &'arena str,
        arena: &'arena bumpalo::Bump,
    ) -> Result<HVector<'arena>, VectorError> {
        let vector_data_bytes = self
            .vectors_db
            .get(txn, &Self::vector_key(id, 0))?
            .ok_or(VectorError::EntryPointNotFound)?;
        HVector::from_raw_vector_data(arena, vector_data_bytes, label, id)
    }

    /// Returns the count of vectors reachable by BFS from the entry point at level 0.
    /// Soft-deleted neighbors are visited (to continue traversal) but not counted.
    pub fn bfs_reachable_count<'db: 'arena, 'arena>(
        &self,
        txn: &'arena heed3::RoTxn<'db>,
        label: &'arena str,
        arena: &'arena bumpalo::Bump,
    ) -> Result<usize, VectorError> {
        let entry_point = match self.get_entry_point(txn, label, arena) {
            Ok(ep) => ep,
            Err(VectorError::EntryPointNotFound) => return Ok(0),
            Err(e) => return Err(e),
        };

        let mut visited: std::collections::HashSet<u128> = std::collections::HashSet::new();
        let mut queue: std::collections::VecDeque<u128> = std::collections::VecDeque::new();

        visited.insert(entry_point.id);
        queue.push_back(entry_point.id);

        while let Some(id) = queue.pop_front() {
            let neighbors = self.get_neighbors::<fn(&HVector, &heed3::RoTxn) -> bool>(
                txn, label, id, 0, None, arena,
            )?;
            for neighbor in neighbors {
                if visited.insert(neighbor.id) {
                    let is_deleted = self
                        .vector_properties_db
                        .get(txn, &neighbor.id)
                        .ok()
                        .flatten()
                        .and_then(|bytes| {
                            VectorWithoutData::from_bincode_bytes(arena, bytes, neighbor.id).ok()
                        })
                        .map(|p| p.deleted)
                        .unwrap_or(false);
                    if !is_deleted {
                        queue.push_back(neighbor.id);
                    }
                }
            }
        }

        Ok(visited.len())
    }

    /// Returns the count of active (non-deleted) vectors reachable by BFS from the global
    /// entry point at level 0, traversing edges directly without requiring a label.
    /// This covers all labels in a single pass.
    pub fn bfs_reachable_count_global<'db>(
        &self,
        txn: &RoTxn<'db>,
    ) -> Result<usize, VectorError> {
        let ep_bytes = match self.vectors_db.get(txn, ENTRY_POINT_KEY)? {
            None => return Ok(0),
            Some(b) => b,
        };
        let mut arr = [0u8; 16];
        let len = ep_bytes.len().min(16);
        arr[..len].copy_from_slice(&ep_bytes[..len]);
        let ep_id = u128::from_be_bytes(arr);

        let arena = bumpalo::Bump::new();
        let mut visited: HashSet<u128> = HashSet::new();
        let mut queue: std::collections::VecDeque<u128> = std::collections::VecDeque::new();
        let mut active_reachable = 0usize;

        visited.insert(ep_id);
        queue.push_back(ep_id);

        while let Some(id) = queue.pop_front() {
            let is_deleted = self
                .vector_properties_db
                .get(txn, &id)
                .ok()
                .flatten()
                .and_then(|bytes| VectorWithoutData::from_bincode_bytes(&arena, bytes, id).ok())
                .map(|p| p.deleted)
                .unwrap_or(true);

            if !is_deleted {
                active_reachable += 1;
            }

            let prefix = Self::out_edges_key(id, 0, None);
            for result in self.edges_db.prefix_iter(txn, &prefix)? {
                let (key, _) = result?;
                if key.len() < 40 {
                    continue;
                }
                let mut narr = [0u8; 16];
                narr.copy_from_slice(&key[24..40]);
                let neighbor_id = u128::from_be_bytes(narr);
                if visited.insert(neighbor_id) {
                    queue.push_back(neighbor_id);
                }
            }
        }

        Ok(active_reachable)
    }

    /// Returns count of non-deleted vectors for a specific label.
    pub fn count_active_vectors<'db: 'arena, 'arena>(
        &self,
        txn: &'arena heed3::RoTxn<'db>,
        label: &'arena str,
        arena: &'arena bumpalo::Bump,
    ) -> Result<usize, VectorError> {
        let mut count = 0usize;
        let iter = self.vector_properties_db.iter(txn)?;
        for result in iter {
            let (id, bytes) = result?;
            let props = VectorWithoutData::from_bincode_bytes(arena, bytes, id)
                .map_err(|e| VectorError::VectorCoreError(e.to_string()))?;
            if !props.deleted && props.label == label {
                count += 1;
            }
        }
        Ok(count)
    }

    /// Get all vectors from the database, optionally filtered by level
    pub fn get_all_vectors<'db: 'arena, 'arena: 'txn, 'txn>(
        &self,
        txn: &'txn RoTxn<'db>,
        level: Option<usize>,
        arena: &'arena bumpalo::Bump,
    ) -> Result<bumpalo::collections::Vec<'arena, HVector<'arena>>, VectorError> {
        let mut vectors = bumpalo::collections::Vec::new_in(arena);

        // Iterate over all vectors in the database
        let prefix_iter = self.vectors_db.prefix_iter(txn, VECTOR_PREFIX)?;

        for result in prefix_iter {
            let (key, _) = result?;

            // Extract id from the key: v: (2 bytes) + id (16 bytes) + level (8 bytes)
            if key.len() < VECTOR_PREFIX.len() + 16 {
                continue; // Skip malformed keys
            }

            let mut id_bytes = [0u8; 16];
            id_bytes.copy_from_slice(&key[VECTOR_PREFIX.len()..VECTOR_PREFIX.len() + 16]);
            let id = u128::from_be_bytes(id_bytes);

            // Get the full vector using the existing method
            match self.get_full_vector(txn, id, arena) {
                Ok(vector) => {
                    // Filter by level if specified
                    if let Some(lvl) = level {
                        if vector.level == lvl {
                            vectors.push(vector);
                        }
                    } else {
                        vectors.push(vector);
                    }
                }
                Err(_) => {
                    // Skip vectors that can't be loaded (e.g., deleted)
                    continue;
                }
            }
        }

        Ok(vectors)
    }
}

pub struct RebuildStats {
    pub kept: u64,
    pub purged_deleted: u64,
}

impl VectorCore {
    /// Clears all vector data (vectors, edges, properties) and re-inserts every
    /// non-deleted vector with its original ID.  Soft-deleted vectors are dropped.
    pub fn rebuild<'db>(
        &'db self,
        txn: &mut RwTxn<'db>,
        arena: &'db bumpalo::Bump,
    ) -> Result<RebuildStats, VectorError> {
        // Phase 1: Collect all non-deleted vectors as owned data.
        // We own everything (Vec<f64>, String, Vec<u8>) so the borrow of `txn` ends here.
        // The raw properties bytes are captured so that per-vector metadata (custom
        // properties) survives the clear+reinsert cycle without needing to reconstruct
        // an ImmutablePropertiesMap from scratch.
        let mut to_reinsert: Vec<(u128, Vec<f64>, String, Vec<u8>)> = Vec::new();
        let mut purged: u64 = 0;

        {
            let iter = self.vector_properties_db.iter(txn)?;
            for result in iter {
                let (id_key, bytes) = result?;
                let id = id_key;
                let props = VectorWithoutData::from_bincode_bytes(arena, bytes, id)
                    .map_err(|e| VectorError::VectorCoreError(e.to_string()))?;
                if props.deleted {
                    purged += 1;
                    continue;
                }
                // Capture a snapshot of the raw bytes stored in vector_properties_db.
                // These bytes encode (label, version, deleted=false, level, properties)
                // via bincode.  We will restore them verbatim after insert_with_id so
                // that custom per-vector properties are not silently dropped.
                let props_bytes_owned: Vec<u8> = bytes.to_vec();
                let data_key = Self::vector_key(id, 0);
                let data_bytes = self
                    .vectors_db
                    .get(txn, data_key.as_ref())?
                    .map(|b| b.to_vec())
                    .ok_or_else(|| VectorError::VectorNotFound(id.to_string()))?;
                // Data is stored with bytemuck::cast_slice (native endianness).
                let data_f64: Vec<f64> = bytemuck::cast_slice::<u8, f64>(&data_bytes).to_vec();
                to_reinsert.push((id, data_f64, props.label.to_string(), props_bytes_owned));
            }
        }

        // Phase 2: Clear all three tables.
        let all_vector_keys: Vec<Vec<u8>> = self
            .vectors_db
            .iter(txn)?
            .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
            .collect();
        for k in all_vector_keys {
            self.vectors_db.delete(txn, k.as_ref())?;
        }

        let all_edge_keys: Vec<Vec<u8>> = self
            .edges_db
            .iter(txn)?
            .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
            .collect();
        for k in all_edge_keys {
            self.edges_db.delete(txn, k.as_ref())?;
        }

        let all_prop_keys: Vec<u128> = self
            .vector_properties_db
            .iter(txn)?
            .filter_map(|r| r.ok().map(|(k, _)| k))
            .collect();
        for k in all_prop_keys {
            self.vector_properties_db.delete(txn, &k)?;
        }

        // Phase 3: Re-insert each active vector with its original ID.
        // insert_with_id writes a fresh vector_properties_db entry with no custom
        // properties.  After the call we overwrite that entry with the snapshot
        // captured in Phase 1 so that per-vector metadata is fully preserved.
        let kept = to_reinsert.len() as u64;
        for (id, data, label, original_props_bytes) in to_reinsert {
            let data_arena: &[f64] = arena.alloc_slice_copy(&data);
            let label_arena: &str = arena.alloc_str(&label);
            self.insert_with_id::<fn(&_, &_) -> bool>(
                txn, id, label_arena, data_arena, None, arena,
            )?;
            // Restore the original properties bytes (which include custom metadata
            // and have deleted=false, the correct state for an active vector).
            self.vector_properties_db
                .put(txn, &id, original_props_bytes.as_ref())?;
        }

        Ok(RebuildStats {
            kept,
            purged_deleted: purged,
        })
    }

    /// Thin wrapper around `rebuild` — removes soft-deleted vectors from the index.
    pub fn purge_soft_deleted<'db>(
        &'db self,
        txn: &mut RwTxn<'db>,
        arena: &'db bumpalo::Bump,
    ) -> Result<RebuildStats, VectorError> {
        self.rebuild(txn, arena)
    }
}

#[cfg(test)]
mod prune_tests {
    use super::*;
    use bumpalo::Bump;
    use heed3::EnvOpenOptions;

    fn setup_env() -> (heed3::Env, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(64 * 1024 * 1024)
                .max_dbs(16)
                .open(path)
                .unwrap()
        };
        (env, temp_dir)
    }

    /// Write only the raw vector data that `get_raw_vector_data` needs:
    /// vectors_db key = [b"v:", id(16 BE), 0usize(8 BE)], value = f64 bytes
    fn write_vector_data(
        vc: &VectorCore,
        txn: &mut RwTxn,
        id: u128,
        data: &[f64],
    ) {
        let key = VectorCore::vector_key(id, 0);
        let bytes: &[u8] = bytemuck::cast_slice(data);
        vc.vectors_db.put(txn, &key, bytes).unwrap();
    }

    /// Write a directed edge: source_id -> sink_id at the given level.
    fn write_edge(vc: &VectorCore, txn: &mut RwTxn, source: u128, sink: u128, level: usize) {
        let key = VectorCore::out_edges_key(source, level, Some(sink));
        vc.edges_db.put(txn, &key, &()).unwrap();
    }

    /// Count outgoing edges from `node_id` at `level`.
    fn count_edges(vc: &VectorCore, txn: &RoTxn, node_id: u128, level: usize) -> usize {
        let prefix = VectorCore::out_edges_key(node_id, level, None);
        vc.edges_db
            .prefix_iter(txn, &prefix)
            .unwrap()
            .count()
    }

    /// Fabricate a deterministic u128 from an index.
    fn fake_id(i: u64) -> u128 {
        // Use a non-zero base so IDs are spread out and don't collide with 0.
        0x0123_4567_89ab_cdef_0000_0000_0000_0000u128 | (i as u128)
    }

    #[test]
    fn test_prune_if_over_degree_caps_hub_edges() {
        let (env, _tmp) = setup_env();
        let mut txn = env.write_txn().unwrap();

        // Use small m so m_max_0 = 6 for easy over-saturation.
        let config = HNSWConfig::new(Some(3), Some(40), Some(40));
        let vc = VectorCore::new(&env, &mut txn, config).unwrap();

        let m_max_0 = vc.config.m_max_0; // = 6
        let over_count = m_max_0 + 4;    // 10 satellites — 4 more than the limit

        // IDs
        let hub_id = fake_id(0);
        let hub_data = vec![1.0f64, 0.0, 0.0, 0.0];

        // Write hub's raw vector data.
        write_vector_data(&vc, &mut txn, hub_id, &hub_data);

        // Write each satellite's raw vector data and one forward edge hub -> sat.
        for i in 1..=(over_count as u64) {
            let sat_id = fake_id(i);
            let d = 0.001 * i as f64;
            write_vector_data(&vc, &mut txn, sat_id, &[d, d, d, d]);
            write_edge(&vc, &mut txn, hub_id, sat_id, 0);
        }

        // Verify we deliberately seeded more edges than the limit.
        let before = count_edges(&vc, &txn, hub_id, 0);
        assert_eq!(before, over_count, "pre-condition: hub must start over-degree");

        // Build a minimal HVector for hub_id so prune_if_over_degree can compute distances.
        let arena = Bump::new();
        let hub_data_arena = arena.alloc_slice_copy(&hub_data);
        let hub_vec = HVector::from_raw_vector_data(&arena, bytemuck::cast_slice(hub_data_arena), "test", hub_id).unwrap();

        // Call the private method directly (we are in the same module).
        vc.prune_if_over_degree(&mut txn, hub_id, &hub_vec, 0, m_max_0, &arena)
            .unwrap();

        let after = count_edges(&vc, &txn, hub_id, 0);
        assert!(
            after <= m_max_0,
            "after prune: hub has {after} edges but limit is {m_max_0}"
        );
        // Sanity: some edges must have been removed.
        assert!(
            after < before,
            "prune must have removed at least one edge (before={before}, after={after})"
        );

        txn.commit().unwrap();
    }

    /// This test validates the call-site: it pre-seeds a satellite with `m_max_0`
    /// existing neighbours in `edges_db`, then calls `set_neighbours` on the hub with
    /// that satellite listed as a neighbour.  `set_neighbours` writes one more back-link
    /// (hub -> satellite), pushing the satellite over the limit, which must trigger
    /// `prune_if_over_degree` to bring it back down to `m_max_0`.
    ///
    /// If the `prune_if_over_degree` call is removed from `set_neighbours` this test
    /// will fail because the satellite's edge count will exceed `m_max_0`.
    #[test]
    fn test_set_neighbours_triggers_prune_via_back_link() {
        let (env, _tmp) = setup_env();
        let mut txn = env.write_txn().unwrap();

        // m=5, so m_max_0=10.  We'll use m=3 (m_max_0=6) for a compact test.
        let config = HNSWConfig::new(Some(3), Some(40), Some(40));
        let vc = VectorCore::new(&env, &mut txn, config).unwrap();

        let m_max_0 = vc.config.m_max_0; // = 6
        let label = "test";

        // IDs
        let hub_id    = fake_id(100);
        let sat_id    = fake_id(200);

        // Write raw vector data for both.
        write_vector_data(&vc, &mut txn, hub_id, &[1.0, 0.0, 0.0, 0.0]);
        write_vector_data(&vc, &mut txn, sat_id, &[0.001, 0.001, 0.001, 0.001]);

        // Write m_max_0 existing outgoing edges for the satellite so it is already
        // at the degree limit before set_neighbours adds one more back-link.
        for i in 1..=(m_max_0 as u64) {
            let bystander_id = fake_id(1000 + i);
            write_vector_data(&vc, &mut txn, bystander_id, &[0.1 * i as f64; 4]);
            write_edge(&vc, &mut txn, sat_id, bystander_id, 0);
        }

        let before_sat = count_edges(&vc, &txn, sat_id, 0);
        assert_eq!(before_sat, m_max_0, "satellite must start at exactly the degree limit");

        // Build a BinaryHeap containing only the satellite, representing the
        // neighbours we want to assign to the hub at level 0.
        let arena = Bump::new();
        let sat_data_arena = arena.alloc_slice_copy(&[0.001f64, 0.001, 0.001, 0.001]);
        let mut sat_vec = HVector::from_raw_vector_data(&arena, bytemuck::cast_slice(sat_data_arena), label, sat_id).unwrap();
        sat_vec.set_distance(0.001);

        let mut heap = super::super::binary_heap::BinaryHeap::new(&arena);
        heap.push(sat_vec);

        // Call set_neighbours on hub with the satellite as its only neighbour.
        // Internally this writes edge hub->sat AND sat->hub (back-link), pushing
        // sat's degree to m_max_0+1.  prune_if_over_degree should trim it back.
        vc.set_neighbours(&mut txn, hub_id, &heap, 0, &arena).unwrap();

        let after_sat = count_edges(&vc, &txn, sat_id, 0);
        assert!(
            after_sat <= m_max_0,
            "satellite degree after set_neighbours must be <= m_max_0 ({m_max_0}), got {after_sat}"
        );

        txn.commit().unwrap();
    }
}

impl HNSW for VectorCore {
    fn search<'db, 'arena, 'txn, F>(
        &self,
        txn: &'txn RoTxn<'db>,
        query: &'arena [f64],
        k: usize,
        label: &'arena str,
        filter: Option<&'arena [F]>,
        should_trickle: bool,
        arena: &'arena bumpalo::Bump,
    ) -> Result<bumpalo::collections::Vec<'arena, HVector<'arena>>, VectorError>
    where
        F: Fn(&HVector<'arena>, &RoTxn<'db>) -> bool,
        'db: 'arena,
        'arena: 'txn,
    {
        let query = HVector::from_slice(label, 0, query);
        // let temp_arena = bumpalo::Bump::new();

        let mut entry_point = self.get_entry_point(txn, label, arena)?;

        let ef = self.config.ef;
        let curr_level = entry_point.level;
        // println!("curr_level: {curr_level}");
        for level in (1..=curr_level).rev() {
            let mut nearest = self.search_level(
                txn,
                label,
                &query,
                &mut entry_point,
                ef,
                level,
                match should_trickle {
                    true => filter,
                    false => None,
                },
                arena,
            )?;
            if let Some(closest) = nearest.pop() {
                entry_point = closest;
            }
        }
        // println!("entry_point: {entry_point:?}");
        let candidates = self.search_level(
            txn,
            label,
            &query,
            &mut entry_point,
            ef,
            0,
            match should_trickle {
                true => filter,
                false => None,
            },
            arena,
        )?;
        // println!("candidates");
        let results = candidates.to_vec_with_filter::<F, true>(
            k,
            filter,
            label,
            txn,
            self.vector_properties_db,
            arena,
        )?;

        debug_println!("vector search found {} results", results.len());
        Ok(results)
    }

    fn insert<'db, 'arena, 'txn, F>(
        &'db self,
        txn: &'txn mut RwTxn<'db>,
        label: &'arena str,
        data: &'arena [f64],
        properties: Option<ImmutablePropertiesMap<'arena>>,
        arena: &'arena bumpalo::Bump,
    ) -> Result<HVector<'arena>, VectorError>
    where
        F: Fn(&HVector<'arena>, &RoTxn<'db>) -> bool,
        'db: 'arena,
        'arena: 'txn,
    {
        if !data.is_empty() && data.iter().map(|x| x * x).sum::<f64>() == 0.0 {
            return Err(VectorError::ZeroMagnitudeVector);
        }

        let new_level = self.get_new_level();

        let mut query = HVector::from_slice(label, 0, data);
        query.properties = properties;
        self.put_vector(txn, &query)?;

        query.level = new_level;

        let entry_point = match self.get_entry_point(txn, label, arena) {
            Ok(ep) => ep,
            Err(VectorError::EntryPointNotFound) => {
                self.set_entry_point(txn, &query)?;
                query.set_distance(0.0);
                return Ok(query);
            }
            Err(e) => return Err(e),
        };

        let l = entry_point.level;
        let mut curr_ep = entry_point;
        for level in (new_level + 1..=l).rev() {
            let mut nearest =
                self.search_level::<F>(txn, label, &query, &mut curr_ep, 1, level, None, arena)?;
            curr_ep = nearest.pop().ok_or(VectorError::VectorCoreError(
                "emtpy search result".to_string(),
            ))?;
        }

        for level in (0..=l.min(new_level)).rev() {
            let nearest = self.search_level::<F>(
                txn,
                label,
                &query,
                &mut curr_ep,
                self.config.ef_construct,
                level,
                None,
                arena,
            )?;
            curr_ep = *nearest.peek().ok_or(VectorError::VectorCoreError(
                "emtpy search result".to_string(),
            ))?;

            let neighbors =
                self.select_neighbors::<F>(txn, label, &query, nearest, level, true, None, arena)?;
            self.set_neighbours(txn, query.id, &neighbors, level, arena)?;

            for e in neighbors {
                let id = e.id;
                let e_conns = BinaryHeap::from(
                    arena,
                    self.get_neighbors::<F>(txn, label, id, level, None, arena)?,
                );
                let e_new_conn = self
                    .select_neighbors::<F>(txn, label, &query, e_conns, level, true, None, arena)?;
                self.set_neighbours(txn, id, &e_new_conn, level, arena)?;
            }
        }

        if new_level > l {
            self.set_entry_point(txn, &query)?;
        }

        debug_println!("vector inserted with id {}", query.id);
        Ok(query)
    }

    fn delete(&self, txn: &mut RwTxn, id: u128, arena: &bumpalo::Bump) -> Result<(), VectorError> {
        match self.get_vector_properties(txn, id, arena)? {
            Some(mut properties) => {
                debug_println!("properties: {properties:?}");
                if properties.deleted {
                    return Err(VectorError::VectorAlreadyDeleted(id.to_string()));
                }

                properties.deleted = true;
                self.vector_properties_db.put(
                    txn,
                    &id,
                    bincode::serialize(&properties)?.as_ref(),
                )?;
                debug_println!("vector deleted with id {}", &id);

                if let Ok(Some(ep_bytes)) = self.vectors_db.get(txn, ENTRY_POINT_KEY) {
                    let ep_bytes_ref: &[u8] = &ep_bytes;
                    if ep_bytes_ref.len() == 16 {
                        let ep_id = u128::from_be_bytes(ep_bytes_ref.try_into().unwrap());
                        if ep_id == id {
                            let edge_prefix = Self::out_edges_key(id, 0, None);
                            let neighbor_ids: Vec<u128> = self
                                .edges_db
                                .prefix_iter(txn, edge_prefix.as_ref())?
                                .filter_map(|r| r.ok())
                                .filter_map(|(key, _)| {
                                    if key.len() == 40 {
                                        let mut arr = [0u8; 16];
                                        arr.copy_from_slice(&key[24..40]);
                                        Some(u128::from_be_bytes(arr))
                                    } else {
                                        None
                                    }
                                })
                                .collect();

                            let label = properties.label;
                            let mut replacement = None;
                            for neighbor_id in neighbor_ids {
                                let props_bytes = self
                                    .vector_properties_db
                                    .get(txn, &neighbor_id)
                                    .ok()
                                    .flatten();
                                let is_deleted = props_bytes
                                    .and_then(|b| {
                                        VectorWithoutData::from_bincode_bytes(arena, b, neighbor_id).ok()
                                    })
                                    .map(|p| p.deleted)
                                    .unwrap_or(true);
                                if !is_deleted {
                                    if let Ok(v) = self.get_raw_vector_data(txn, neighbor_id, label, arena) {
                                        replacement = Some(v);
                                        break;
                                    }
                                }
                            }

                            match replacement {
                                Some(new_ep) => self.set_entry_point(txn, &new_ep)?,
                                None => {
                                    self.vectors_db.delete(txn, ENTRY_POINT_KEY)?;
                                }
                            }
                        }
                    }
                }

                Ok(())
            }
            None => Err(VectorError::VectorNotFound(id.to_string())),
        }
    }

    fn hard_delete(&self, txn: &mut RwTxn, id: u128) -> Result<(), VectorError> {
        let data_prefix = [VECTOR_PREFIX, id.to_be_bytes().as_ref()].concat();
        let data_keys: Vec<Vec<u8>> = self
            .vectors_db
            .prefix_iter(txn, data_prefix.as_ref())?
            .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
            .collect();
        for key in data_keys {
            self.vectors_db.delete(txn, key.as_ref())?;
        }

        let _ = self.vector_properties_db.delete(txn, &id);

        let forward_keys: Vec<Vec<u8>> = self
            .edges_db
            .prefix_iter(txn, id.to_be_bytes().as_ref())?
            .filter_map(|r| r.ok().map(|(k, _)| k.to_vec()))
            .collect();
        for fwd in &forward_keys {
            if fwd.len() == 40 {
                let mut rev = [0u8; 40];
                rev[..16].copy_from_slice(&fwd[24..40]);
                rev[16..24].copy_from_slice(&fwd[16..24]);
                rev[24..40].copy_from_slice(&fwd[..16]);
                let _ = self.edges_db.delete(txn, rev.as_ref());
            }
            self.edges_db.delete(txn, fwd.as_ref())?;
        }

        if let Ok(Some(ep_bytes)) = self.vectors_db.get(txn, ENTRY_POINT_KEY) {
            let ep_bytes_ref: &[u8] = &ep_bytes;
            if ep_bytes_ref.len() == 16 {
                let ep_id = u128::from_be_bytes(ep_bytes_ref.try_into().unwrap());
                if ep_id == id {
                    self.vectors_db.delete(txn, ENTRY_POINT_KEY)?;
                }
            }
        }

        Ok(())
    }

    fn insert_with_id<'db, 'arena, 'txn, F>(
        &'db self,
        txn: &'txn mut RwTxn<'db>,
        id: u128,
        label: &'arena str,
        data: &'arena [f64],
        properties: Option<ImmutablePropertiesMap<'arena>>,
        arena: &'arena bumpalo::Bump,
    ) -> Result<HVector<'arena>, VectorError>
    where
        F: Fn(&HVector<'arena>, &RoTxn<'db>) -> bool,
        'db: 'arena,
        'arena: 'txn,
    {
        let new_level = self.get_new_level();

        let mut query = HVector {
            id,
            label,
            version: 1,
            deleted: false,
            level: 0,
            distance: None,
            data,
            properties,
        };
        self.put_vector(txn, &query)?;

        query.level = new_level;

        let entry_point = match self.get_entry_point(txn, label, arena) {
            Ok(ep) => ep,
            Err(_) => {
                self.set_entry_point(txn, &query)?;
                query.set_distance(0.0);
                return Ok(query);
            }
        };

        let l = entry_point.level;
        let mut curr_ep = entry_point;
        for level in (new_level + 1..=l).rev() {
            let mut nearest =
                self.search_level::<F>(txn, label, &query, &mut curr_ep, 1, level, None, arena)?;
            curr_ep = nearest.pop().ok_or(VectorError::VectorCoreError(
                "emtpy search result".to_string(),
            ))?;
        }

        for level in (0..=l.min(new_level)).rev() {
            let nearest = self.search_level::<F>(
                txn,
                label,
                &query,
                &mut curr_ep,
                self.config.ef_construct,
                level,
                None,
                arena,
            )?;
            curr_ep = *nearest.peek().ok_or(VectorError::VectorCoreError(
                "emtpy search result".to_string(),
            ))?;

            let neighbors =
                self.select_neighbors::<F>(txn, label, &query, nearest, level, true, None, arena)?;
            self.set_neighbours(txn, query.id, &neighbors, level, arena)?;

            for e in neighbors {
                let id = e.id;
                let e_conns = BinaryHeap::from(
                    arena,
                    self.get_neighbors::<F>(txn, label, id, level, None, arena)?,
                );
                let e_new_conn = self
                    .select_neighbors::<F>(txn, label, &query, e_conns, level, true, None, arena)?;
                self.set_neighbours(txn, id, &e_new_conn, level, arena)?;
            }
        }

        if new_level > l {
            self.set_entry_point(txn, &query)?;
        }

        debug_println!("vector inserted with id {}", query.id);
        Ok(query)
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;
    use heed3::EnvOpenOptions;

    fn setup_env() -> (heed3::Env, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();
        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(64 * 1024 * 1024)
                .max_dbs(16)
                .open(path)
                .unwrap()
        };
        (env, temp_dir)
    }

    #[test]
    fn test_vector_stats_counts_correctly() {
        let (env, _tmp) = setup_env();
        let mut wtxn = env.write_txn().unwrap();
        let config = HNSWConfig::new(None, None, None);
        let vc = VectorCore::new(&env, &mut wtxn, config).unwrap();
        let arena = bumpalo::Bump::new();

        let dim = 4;
        let mut v1_data = vec![0.0f64; dim];
        v1_data[0] = 1.0;
        let v1 = vc
            .insert::<fn(&_, &_) -> bool>(&mut wtxn, "test", &v1_data, None, &arena)
            .unwrap();

        let mut v2_data = vec![0.0f64; dim];
        v2_data[1] = 1.0;
        let _v2 = vc
            .insert::<fn(&_, &_) -> bool>(&mut wtxn, "test", &v2_data, None, &arena)
            .unwrap();

        vc.delete(&mut wtxn, v1.id, &arena).unwrap();

        let stats = vc.stats(&wtxn).unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.active, 1);
        assert_eq!(stats.soft_deleted, 1);
        assert!(stats.entry_point_present);

        wtxn.commit().unwrap();
    }

    #[test]
    fn test_vector_stats_empty() {
        let (env, _tmp) = setup_env();
        let mut wtxn = env.write_txn().unwrap();
        let config = HNSWConfig::new(None, None, None);
        let vc = VectorCore::new(&env, &mut wtxn, config).unwrap();
        wtxn.commit().unwrap();

        let rtxn = env.read_txn().unwrap();
        let stats = vc.stats(&rtxn).unwrap();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.active, 0);
        assert_eq!(stats.soft_deleted, 0);
        assert_eq!(stats.hnsw_edges, 0);
        assert!(!stats.entry_point_present);
    }
}
