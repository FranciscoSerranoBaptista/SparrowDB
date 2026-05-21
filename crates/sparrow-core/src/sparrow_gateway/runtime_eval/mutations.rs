use crate::{
    sparrow_engine::{
        storage_core::{storage_methods::StorageMethods, SparrowGraphStorage},
        traversal_core::{
            ops::{
                g::G,
                source::{add_e::AddEAdapter, add_n::AddNAdapter},
                util::update::UpdateAdapter,
            },
            traversal_value::TraversalValue,
        },
        types::GraphError,
    },
    sparrow_gateway::mcp::tools::{execute_query_chain, execute_query_chain_from_seed},
    utils::properties::ImmutablePropertiesMap,
    protocol::value::Value,
};
use bumpalo::Bump;
use std::collections::HashMap;
use super::{lower::MutationOp, RuntimeError};

pub fn execute_mutation<'db, 'arena>(
    op: &MutationOp,
    _bind_to: &str,
    live_store: &mut HashMap<String, Vec<TraversalValue<'arena>>>,
    storage: &'db SparrowGraphStorage,
    arena: &'arena Bump,
) -> Result<Vec<TraversalValue<'arena>>, RuntimeError>
where
    'db: 'arena,
{
    match op {
        MutationOp::AddNode { node_type, fields } => {
            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            let label: &'arena str = arena.alloc_str(node_type);

            // Collect secondary index names for fields that have indices registered.
            let sec_index_names: Vec<String> = fields
                .keys()
                .filter(|k| storage.secondary_indices.contains_key(k.as_str()))
                .cloned()
                .collect();
            // Leak each name to produce &'static str — acceptable for a debug/eval tool.
            let static_sec_names: Vec<&'static str> = sec_index_names
                .iter()
                .map(|k| Box::leak(k.clone().into_boxed_str()) as &'static str)
                .collect();
            let sec_indices: Option<&[&str]> = if static_sec_names.is_empty() {
                None
            } else {
                Some(&static_sec_names)
            };

            // Build ImmutablePropertiesMap
            let fields_count = fields.len();
            let fields_iter = fields
                .iter()
                .map(|(k, v)| (arena.alloc_str(k) as &'arena str, v.clone()));
            let props = ImmutablePropertiesMap::new(fields_count, fields_iter, arena);

            let result = G::new_mut(storage, arena, &mut wtxn)
                .add_n(label, Some(props), sec_indices)
                .collect_to_obj()
                .map_err(RuntimeError::Execution)?;

            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            Ok(vec![result])
        }

        MutationOp::AddEdge {
            edge_type,
            from_var,
            to_var,
            fields,
        } => {
            let from_nodes = live_store
                .get(from_var)
                .ok_or_else(|| RuntimeError::Lowering(format!("variable '{from_var}' not found")))?;
            if from_nodes.len() != 1 {
                return Err(RuntimeError::Lowering(format!(
                    "AddEdge From variable '{from_var}' must contain exactly one node, got {}",
                    from_nodes.len()
                )));
            }
            let from_tv = &from_nodes[0];
            let from_id = match from_tv {
                TraversalValue::Node(n) => n.id,
                _ => return Err(RuntimeError::Lowering(format!(
                    "AddEdge From variable '{from_var}' does not contain a node"
                ))),
            };

            let to_nodes = live_store
                .get(to_var)
                .ok_or_else(|| RuntimeError::Lowering(format!("variable '{to_var}' not found")))?;
            if to_nodes.len() != 1 {
                return Err(RuntimeError::Lowering(format!(
                    "AddEdge To variable '{to_var}' must contain exactly one node, got {}",
                    to_nodes.len()
                )));
            }
            let to_tv = &to_nodes[0];
            let to_id = match to_tv {
                TraversalValue::Node(n) => n.id,
                _ => return Err(RuntimeError::Lowering(format!(
                    "AddEdge To variable '{to_var}' does not contain a node"
                ))),
            };

            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            let label: &'arena str = arena.alloc_str(edge_type);

            let props = if fields.is_empty() {
                None
            } else {
                let fields_count = fields.len();
                let fields_iter = fields
                    .iter()
                    .map(|(k, v)| (arena.alloc_str(k) as &'arena str, v.clone()));
                Some(ImmutablePropertiesMap::new(fields_count, fields_iter, arena))
            };

            let result = G::new_mut(storage, arena, &mut wtxn)
                .add_edge(label, props, from_id, to_id, false)
                .collect_to_obj()
                .map_err(RuntimeError::Execution)?;

            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            Ok(vec![result])
        }

        MutationOp::DropNodes {
            seed_var,
            tool_args,
        } => {
            // Phase 1: Traverse (read txn) to find targets.
            let targets: Vec<TraversalValue<'arena>> = {
                let rtxn = storage
                    .graph_env
                    .read_txn()
                    .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
                if let Some(seed_name) = seed_var {
                    let seeds = live_store.get(seed_name).cloned().unwrap_or_default();
                    execute_query_chain_from_seed(
                        tool_args,
                        storage,
                        &rtxn,
                        arena,
                        seeds.into_iter(),
                    )
                    .map_err(RuntimeError::Execution)?
                    .collect()
                    .map_err(RuntimeError::Execution)?
                } else {
                    execute_query_chain(tool_args, storage, &rtxn, arena)
                        .map_err(RuntimeError::Execution)?
                        .collect()
                        .map_err(RuntimeError::Execution)?
                }
            }; // rtxn dropped here

            // Phase 2: Write txn to drop each target.
            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            for target in &targets {
                match target {
                    TraversalValue::Node(node) => storage
                        .drop_node(&mut wtxn, node.id)
                        .map_err(RuntimeError::Execution)?,
                    TraversalValue::Edge(edge) => storage
                        .drop_edge(&mut wtxn, edge.id)
                        .map_err(RuntimeError::Execution)?,
                    _ => {}
                }
            }

            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            Ok(vec![])
        }

        MutationOp::UpdateNodes {
            seed_var,
            tool_args,
            updates,
        } => {
            // Phase 1: Traverse (read txn) to find targets.
            let targets: Vec<TraversalValue<'arena>> = {
                let rtxn = storage
                    .graph_env
                    .read_txn()
                    .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;
                if let Some(seed_name) = seed_var {
                    let seeds = live_store.get(seed_name).cloned().unwrap_or_default();
                    execute_query_chain_from_seed(
                        tool_args,
                        storage,
                        &rtxn,
                        arena,
                        seeds.into_iter(),
                    )
                    .map_err(RuntimeError::Execution)?
                    .collect()
                    .map_err(RuntimeError::Execution)?
                } else {
                    execute_query_chain(tool_args, storage, &rtxn, arena)
                        .map_err(RuntimeError::Execution)?
                        .collect()
                        .map_err(RuntimeError::Execution)?
                }
            }; // rtxn dropped here

            // Leak field name strings to satisfy &'static str requirement of update().
            // Memory bounded by distinct field names per request — acceptable for a debug tool.
            let static_props: Vec<(&'static str, Value)> = updates
                .iter()
                .map(|(k, v)| {
                    let static_key: &'static str = Box::leak(k.clone().into_boxed_str());
                    (static_key, v.clone())
                })
                .collect();

            // Phase 2: Write txn to update each target node.
            let mut wtxn = storage
                .graph_env
                .write_txn()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            let mut updated_nodes = Vec::new();
            for target in targets {
                let updated = G::new_mut_from(storage, &mut wtxn, target, arena)
                    .update(&static_props)
                    .collect_to_obj()
                    .map_err(RuntimeError::Execution)?;
                updated_nodes.push(updated);
            }

            wtxn.commit()
                .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

            Ok(updated_nodes)
        }
    }
}
