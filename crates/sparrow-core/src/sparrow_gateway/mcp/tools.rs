use crate::{
    protocol::value::Value,
    sparrow_engine::{
        storage_core::{SparrowGraphStorage, Txn},
        traversal_core::{
            ops::{
                g::G,
                in_::{in_::InAdapter, in_e::InEdgesAdapter},
                out::{out::OutAdapter, out_e::OutEdgesAdapter},
                source::{e_from_type::EFromTypeAdapter, n_from_type::NFromTypeAdapter},
                util::{order::OrderByAdapter, range::RangeAdapter},
            },
            traversal_iter::RoTraversalIterator,
            traversal_value::TraversalValue,
        },
        types::GraphError,
    },
};
use bumpalo::Bump;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Node,
    Vec,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "tool_name", content = "args")]
pub enum ToolArgs {
    OutStep {
        edge_label: String,
        edge_type: EdgeType,
        filter: Option<FilterTraversal>,
    },
    OutEStep {
        edge_label: String,
        filter: Option<FilterTraversal>,
    },
    InStep {
        edge_label: String,
        edge_type: EdgeType,
        filter: Option<FilterTraversal>,
    },
    InEStep {
        edge_label: String,
        filter: Option<FilterTraversal>,
    },
    NFromType {
        node_type: String,
    },
    EFromType {
        edge_type: String,
    },
    FilterItems {
        #[serde(default)]
        filter: FilterTraversal,
    },
    OrderBy {
        properties: String,
        order: Order,
    },
    SearchKeyword {
        query: String,
        limit: usize,
        label: String,
    },
    SearchVecText {
        query: String,
        label: String,
        k: usize,
    },
    SearchVec {
        vector: Vec<f64>,
        k: usize,
        min_score: Option<f64>,
    },
    /// Truncate the traversal stream to at most `n` items (0-based range [0, n)).
    Limit {
        n: usize,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum Order {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FilterProperties {
    pub key: String,
    pub value: Value,
    pub operator: Option<Operator>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FilterTraversal {
    pub properties: Option<Vec<Vec<FilterProperties>>>,
    pub filter_traversals: Option<Vec<ToolArgs>>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum Operator {
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Neq,
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = ">=")]
    Gte,
    #[serde(rename = "<=")]
    Lte,
}

impl Operator {
    #[inline]
    pub fn execute(&self, lhs: &Value, rhs: &Value) -> bool {
        match self {
            Operator::Eq => lhs == rhs,
            Operator::Neq => lhs != rhs,
            Operator::Gt => lhs > rhs,
            Operator::Lt => lhs < rhs,
            Operator::Gte => lhs >= rhs,
            Operator::Lte => lhs <= rhs,
        }
    }
}

type DynIter<'arena, 'txn> =
    Box<dyn Iterator<Item = Result<TraversalValue<'arena>, GraphError>> + 'txn>;

pub struct TraversalStream<'db, 'arena, 'txn>
where
    'db: 'arena,
    'arena: 'txn,
{
    iter: RoTraversalIterator<'db, 'arena, 'txn, DynIter<'arena, 'txn>>,
}

impl<'db, 'arena, 'txn> TraversalStream<'db, 'arena, 'txn>
where
    'db: 'arena,
    'arena: 'txn,
{
    pub fn new(
        storage: &'db SparrowGraphStorage,
        txn: &'txn Txn<'db>,
        arena: &'arena Bump,
    ) -> Self {
        Self::from_ro_iterator(G::new(storage, txn, arena))
    }

    pub fn from_iter(
        storage: &'db SparrowGraphStorage,
        txn: &'txn Txn<'db>,
        arena: &'arena Bump,
        items: impl Iterator<Item = TraversalValue<'arena>> + 'txn,
    ) -> Self {
        Self::from_ro_iterator(G::from_iter(storage, txn, items, arena))
    }

    pub fn from_ro_iterator<I>(iter: RoTraversalIterator<'db, 'arena, 'txn, I>) -> Self
    where
        I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>> + 'txn,
    {
        let RoTraversalIterator {
            storage,
            arena,
            txn,
            inner,
        } = iter;

        let boxed: DynIter<'arena, 'txn> = Box::new(inner);

        Self {
            iter: RoTraversalIterator {
                storage,
                arena,
                txn,
                inner: boxed,
            },
        }
    }

    pub fn map<I, F>(self, f: F) -> TraversalStream<'db, 'arena, 'txn>
    where
        I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>> + 'txn,
        F: FnOnce(
            RoTraversalIterator<'db, 'arena, 'txn, DynIter<'arena, 'txn>>,
        ) -> RoTraversalIterator<'db, 'arena, 'txn, I>,
    {
        TraversalStream::from_ro_iterator(f(self.iter))
    }

    pub fn into_ro(self) -> RoTraversalIterator<'db, 'arena, 'txn, DynIter<'arena, 'txn>> {
        self.iter
    }

    pub fn into_inner_iter(self) -> DynIter<'arena, 'txn> {
        self.iter.inner
    }

    pub fn collect(self) -> Result<Vec<TraversalValue<'arena>>, GraphError> {
        let mut values = Vec::new();
        for item in self.into_inner_iter() {
            values.push(item?);
        }
        Ok(values)
    }

    pub fn nth(self, index: usize) -> Result<Option<TraversalValue<'arena>>, GraphError> {
        let mut iter = self.into_inner_iter();
        for _ in 0..index {
            if let Some(res) = iter.next() {
                res?;
            } else {
                return Ok(None);
            }
        }

        match iter.next() {
            Some(res) => res.map(Some),
            None => Ok(None),
        }
    }
}

pub fn execute_query_chain<'db, 'arena, 'txn>(
    steps: &[ToolArgs],
    storage: &'db SparrowGraphStorage,
    txn: &'txn Txn<'db>,
    arena: &'arena Bump,
) -> Result<TraversalStream<'db, 'arena, 'txn>, GraphError>
where
    'db: 'arena,
    'arena: 'txn,
{
    let initial = TraversalStream::new(storage, txn, arena);
    execute_query_chain_with_stream(initial, steps, storage, txn, arena)
}

pub fn execute_query_chain_from_seed<'db, 'arena, 'txn>(
    steps: &[ToolArgs],
    storage: &'db SparrowGraphStorage,
    txn: &'txn Txn<'db>,
    arena: &'arena Bump,
    seed: impl Iterator<Item = TraversalValue<'arena>> + 'txn,
) -> Result<TraversalStream<'db, 'arena, 'txn>, GraphError>
where
    'db: 'arena,
    'arena: 'txn,
{
    let initial = TraversalStream::from_iter(storage, txn, arena, seed);
    execute_query_chain_with_stream(initial, steps, storage, txn, arena)
}

pub fn execute_query_chain_with_stream<'db, 'arena, 'txn>(
    initial: TraversalStream<'db, 'arena, 'txn>,
    steps: &[ToolArgs],
    storage: &'db SparrowGraphStorage,
    txn: &'txn Txn<'db>,
    arena: &'arena Bump,
) -> Result<TraversalStream<'db, 'arena, 'txn>, GraphError>
where
    'db: 'arena,
    'arena: 'txn,
{
    steps.iter().try_fold(initial, |stream, step| {
        apply_step(stream, step, storage, txn, arena)
    })
}

fn apply_step<'db, 'arena, 'txn>(
    stream: TraversalStream<'db, 'arena, 'txn>,
    step: &ToolArgs,
    storage: &'db SparrowGraphStorage,
    txn: &'txn Txn<'db>,
    arena: &'arena Bump,
) -> Result<TraversalStream<'db, 'arena, 'txn>, GraphError>
where
    'db: 'arena,
    'arena: 'txn,
{
    match step {
        ToolArgs::NFromType { node_type } => {
            let label = arena.alloc_str(node_type);
            Ok(TraversalStream::from_ro_iterator(
                G::new(storage, txn, arena).n_from_type(label),
            ))
        }
        ToolArgs::EFromType { edge_type } => {
            let label = arena.alloc_str(edge_type);
            Ok(TraversalStream::from_ro_iterator(
                G::new(storage, txn, arena).e_from_type(label),
            ))
        }
        ToolArgs::OutStep {
            edge_label,
            edge_type,
            filter,
        } => {
            let label = arena.alloc_str(edge_label);
            let edge_kind = *edge_type;
            let transformed = match edge_kind {
                EdgeType::Node => stream.map(|iter| iter.out_node(label)),
                EdgeType::Vec => stream.map(|iter| iter.out_vec(label, true)),
            };

            if let Some(filter) = filter.clone() {
                apply_filter(transformed, filter)
            } else {
                Ok(transformed)
            }
        }
        ToolArgs::OutEStep { edge_label, filter } => {
            let label = arena.alloc_str(edge_label);
            let transformed = stream.map(|iter| iter.out_e(label));

            if let Some(filter) = filter.clone() {
                apply_filter(transformed, filter)
            } else {
                Ok(transformed)
            }
        }
        ToolArgs::InStep {
            edge_label,
            edge_type,
            filter,
        } => {
            let label = arena.alloc_str(edge_label);
            let edge_kind = *edge_type;
            let transformed = match edge_kind {
                EdgeType::Node => stream.map(|iter| iter.in_node(label)),
                EdgeType::Vec => stream.map(|iter| iter.in_vec(label, true)),
            };

            if let Some(filter) = filter.clone() {
                apply_filter(transformed, filter)
            } else {
                Ok(transformed)
            }
        }
        ToolArgs::InEStep { edge_label, filter } => {
            let label = arena.alloc_str(edge_label);
            let transformed = stream.map(|iter| iter.in_e(label));

            if let Some(filter) = filter.clone() {
                apply_filter(transformed, filter)
            } else {
                Ok(transformed)
            }
        }
        ToolArgs::FilterItems { filter } => apply_filter(stream, filter.clone()),
        ToolArgs::OrderBy { properties, order } => {
            let props = arena.alloc_str(properties);
            let values = stream.collect()?;
            let iter = TraversalStream::from_iter(storage, txn, arena, values.into_iter());
            let ordered_stream = match order {
                Order::Asc => iter.map(|iter| iter.order_by_asc(props)),
                Order::Desc => iter.map(|iter| iter.order_by_desc(props)),
            };
            Ok(ordered_stream)
        }
        ToolArgs::Limit { n } => {
            let limit = *n;
            Ok(stream.map(move |iter| iter.range(0usize, limit)))
        }
        ToolArgs::SearchKeyword { .. } => {
            // SearchKeyword requires special BM25 indexing and connection state
            // It should be called via the dedicated search_keyword MCP handler
            // not through the generic query chain execution
            Err(GraphError::New(
                "SearchKeyword is not supported in generic query chains. Use the search_keyword endpoint directly.".to_string()
            ))
        }
        ToolArgs::SearchVecText { query, label, k } => {
            // SearchVecText requires embedding model initialization
            // It should be called via the dedicated search_vec_text MCP handler
            // not through the generic query chain execution
            Err(GraphError::New(format!(
                "SearchVecText (query: {}, label: {}, k: {}) is not supported in generic query chains. Use the search_vec_text endpoint directly.",
                query, label, k
            )))
        }
        ToolArgs::SearchVec {
            vector,
            k,
            min_score,
        } => {
            use crate::sparrow_engine::traversal_core::ops::vectors::brute_force_search::BruteForceSearchVAdapter;

            let query_vec = arena.alloc_slice_copy(vector);
            let mut results =
                stream.map(|iter| iter.range(0, *k * 3).brute_force_search_v(query_vec, *k));

            // Apply min_score filter if specified
            if let Some(min_score_val) = min_score {
                let min_score_copy = *min_score_val;
                results = results.map(|iter| {
                    let RoTraversalIterator {
                        storage,
                        arena,
                        txn,
                        inner,
                    } = iter;
                    let filtered: DynIter<'arena, 'txn> = Box::new(inner.filter(move |item_res| {
                        match item_res {
                            Ok(TraversalValue::Vector(v)) => v.get_distance() > min_score_copy,
                            _ => true, // Keep non-vector items
                        }
                    }));
                    RoTraversalIterator {
                        storage,
                        arena,
                        txn,
                        inner: filtered,
                    }
                });
            }

            Ok(results)
        }
    }
}

fn apply_filter<'db, 'arena, 'txn>(
    stream: TraversalStream<'db, 'arena, 'txn>,
    filter: FilterTraversal,
) -> Result<TraversalStream<'db, 'arena, 'txn>, GraphError>
where
    'db: 'arena,
    'arena: 'txn,
{
    let filter_arc = Arc::new(filter);

    Ok(stream.map(|iter| {
        let RoTraversalIterator {
            storage,
            arena,
            txn,
            inner,
        } = iter;

        let filter_clone = Arc::clone(&filter_arc);
        let filtered: DynIter<'arena, 'txn> =
            Box::new(inner.filter_map(move |item_res| match item_res {
                Ok(item) => match matches_filter(&item, &filter_clone, storage, txn, arena) {
                    Ok(true) => Some(Ok(item)),
                    Ok(false) => None,
                    Err(err) => Some(Err(err)),
                },
                Err(err) => Some(Err(err)),
            }));

        RoTraversalIterator {
            storage,
            arena,
            txn,
            inner: filtered,
        }
    }))
}

fn matches_filter<'db, 'arena, 'txn>(
    item: &TraversalValue<'arena>,
    filter: &FilterTraversal,
    storage: &'db SparrowGraphStorage,
    txn: &'txn Txn<'db>,
    arena: &'arena Bump,
) -> Result<bool, GraphError>
where
    'db: 'arena,
    'arena: 'txn,
{
    if !matches_properties(item, filter.properties.as_ref()) {
        return Ok(false);
    }

    match &filter.filter_traversals {
        Some(traversals) => {
            for tool in traversals {
                if !evaluate_sub_traversal(item, tool, storage, txn, arena)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        None => Ok(true),
    }
}

fn matches_properties(
    item: &TraversalValue<'_>,
    groups: Option<&Vec<Vec<FilterProperties>>>,
) -> bool {
    match groups {
        Some(groups) => groups.iter().any(|filters| {
            filters.iter().all(|filter| match filter.key.as_str() {
                // `id` and `label` are struct-level fields, not stored properties.
                // get_property("id") always returns None because the property bag
                // never contains an "id" key — this was the root cause of the
                // Out + Where {Eq: ["$id", ...]} false-negative bug.
                "id" => compare_meta_id(item.id(), &filter.value, filter.operator),
                "label" => compare_meta_label(item.label(), &filter.value, filter.operator),
                _ => item
                    .get_property(&filter.key)
                    .map(|value| value.compare(&filter.value, filter.operator))
                    .unwrap_or(false),
            })
        }),
        None => true,
    }
}

/// Compare a node/edge struct-level `id` (u128) against a filter value.
///
/// The v1_compat DSL sends `$id` as `{"String": "<uuid-str>"}`, so we parse
/// the string back to u128 and compare.  Returns `false` for non-string values
/// or unparseable UUID strings rather than panicking.
fn compare_meta_id(id: u128, filter_value: &Value, operator: Option<Operator>) -> bool {
    let op = operator.unwrap_or(Operator::Eq);
    let Value::String(s) = filter_value else {
        return false;
    };
    let Ok(parsed) = uuid::Uuid::parse_str(s) else {
        return false;
    };
    let parsed_u128 = parsed.as_u128();
    match op {
        Operator::Eq => id == parsed_u128,
        Operator::Neq => id != parsed_u128,
        // Ordering on UUIDs (v6 time-based) is technically defined but not
        // meaningful in this context — reject rather than silently mislead.
        _ => false,
    }
}

/// Compare a node/edge struct-level `label` (&str) against a filter value.
fn compare_meta_label(label: &str, filter_value: &Value, operator: Option<Operator>) -> bool {
    let op = operator.unwrap_or(Operator::Eq);
    let Value::String(s) = filter_value else {
        return false;
    };
    match op {
        Operator::Eq => label == s.as_str(),
        Operator::Neq => label != s.as_str(),
        _ => false,
    }
}

fn evaluate_sub_traversal<'db, 'arena, 'txn>(
    item: &TraversalValue<'arena>,
    step: &ToolArgs,
    storage: &'db SparrowGraphStorage,
    txn: &'txn Txn<'db>,
    arena: &'arena Bump,
) -> Result<bool, GraphError>
where
    'db: 'arena,
    'arena: 'txn,
{
    let seed = std::iter::once(item.clone());
    let stream =
        execute_query_chain_from_seed(std::slice::from_ref(step), storage, txn, arena, seed)?;
    let mut iter = stream.into_inner_iter();
    match iter.next() {
        Some(Ok(_)) => Ok(true),
        Some(Err(err)) => Err(err),
        None => Ok(false),
    }
}

pub trait FilterValues {
    fn compare(&self, value: &Value, operator: Option<Operator>) -> bool;
}
