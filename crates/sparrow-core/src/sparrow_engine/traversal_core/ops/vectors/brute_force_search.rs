use crate::sparrow_engine::{
    traversal_core::{traversal_iter::RoTraversalIterator, traversal_value::TraversalValue},
    types::GraphError,
    vector_core::vector_distance::cosine_similarity,
};
use itertools::Itertools;

pub trait BruteForceSearchVAdapter<'db, 'arena, 'txn>:
    Iterator<Item = Result<TraversalValue<'arena>, GraphError>>
{
    fn brute_force_search_v<K>(
        self,
        query: &'arena [f64],
        k: K,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        K: TryInto<usize>,
        K::Error: std::fmt::Debug;
}

impl<'db, 'arena, 'txn, I: Iterator<Item = Result<TraversalValue<'arena>, GraphError>>>
    BruteForceSearchVAdapter<'db, 'arena, 'txn> for RoTraversalIterator<'db, 'arena, 'txn, I>
{
    fn brute_force_search_v<K>(
        self,
        query: &'arena [f64],
        k: K,
    ) -> RoTraversalIterator<
        'db,
        'arena,
        'txn,
        impl Iterator<Item = Result<TraversalValue<'arena>, GraphError>>,
    >
    where
        K: TryInto<usize>,
        K::Error: std::fmt::Debug,
    {
        // Destructure upfront so each field can be used independently.
        // storage/txn/arena are references (Copy), so they survive the move closure below.
        let RoTraversalIterator { storage, arena, txn, inner } = self;

        let k_res: Result<usize, _> = k.try_into();

        // sorted_by() is already eager; collect to Vec to unify the concrete iterator type
        // across the error and success branches.
        let results: Vec<Result<TraversalValue<'arena>, GraphError>> = match k_res {
            Err(_) => vec![Err(GraphError::New(
                "vector search k must be a non-negative integer".to_string(),
            ))],
            Ok(k_usize) => inner
                .filter_map(|v| match v {
                    Ok(TraversalValue::Vector(mut v)) => {
                        // .ok()? silently skips zero-magnitude stored vectors
                        let d = cosine_similarity(v.data, query).ok()?;
                        v.set_distance(d);
                        Some(v)
                    }
                    _ => None,
                })
                .sorted_by(|v1, v2| v1.partial_cmp(v2).unwrap())
                .take(k_usize)
                .filter_map(move |mut item| {
                    match storage.vectors.get_vector_properties(txn, *item.id(), arena) {
                        Ok(Some(vector_without_data)) => {
                            item.expand_from_vector_without_data(vector_without_data);
                            Some(item)
                        }
                        Ok(None) => None,
                        Err(e) => {
                            println!("error getting vector data: {e:?}");
                            None
                        }
                    }
                })
                .map(|v| Ok(TraversalValue::Vector(v)))
                .collect(),
        };

        RoTraversalIterator {
            storage,
            arena,
            txn,
            inner: results.into_iter(),
        }
    }
}
