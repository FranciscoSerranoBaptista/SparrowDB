use crate::{
    sparrow_engine::{
        storage_core::SparrowGraphStorage,
        traversal_core::traversal_value::TraversalValue,
        types::GraphError,
    },
    sparrow_gateway::mcp::tools::{execute_query_chain, execute_query_chain_from_seed},
};
use bumpalo::Bump;
use std::collections::HashMap;
use super::{
    lower::{LoweredOp, LoweredStep},
    mutations::execute_mutation,
    RuntimeError,
};

/// Execute a lowered plan against storage.
///
/// Each step's results are serialized to `sonic_rs::Value` for the output map.
/// Intermediate `TraversalValue<'arena>` values are also kept alive (tied to the
/// same arena) so they can seed subsequent chained steps without an extra
/// serialise-deserialise round-trip.
pub fn execute_plan(
    ops: &[LoweredOp],
    return_vars: &[String],
    storage: &SparrowGraphStorage,
) -> Result<HashMap<String, Vec<sonic_rs::Value>>, RuntimeError> {
    let arena = Bump::new();
    execute_plan_with_arena(ops, return_vars, storage, &arena)
}

fn execute_plan_with_arena<'db, 'arena>(
    ops: &[LoweredOp],
    return_vars: &[String],
    storage: &'db SparrowGraphStorage,
    arena: &'arena Bump,
) -> Result<HashMap<String, Vec<sonic_rs::Value>>, RuntimeError>
where
    'db: 'arena,
{
    // We store both the serialised output and the live TraversalValues.
    // The live values are needed to seed seed_var-based steps.
    let mut live_store: HashMap<String, Vec<TraversalValue<'arena>>> = HashMap::new();
    let mut json_store: HashMap<String, Vec<sonic_rs::Value>> = HashMap::new();

    for op in ops {
        match op {
            LoweredOp::Traversal(step) => {
                execute_traversal_step(step, storage, arena, &mut live_store, &mut json_store)?;
            }
            LoweredOp::Mutation { bind_to, op: mutation_op } => {
                let result =
                    execute_mutation(mutation_op, bind_to, &mut live_store, storage, arena)?;
                let json_values: Vec<sonic_rs::Value> = result
                    .iter()
                    .map(|v| sonic_rs::to_value(v).unwrap_or_default())
                    .collect();
                live_store.insert(bind_to.clone(), result);
                json_store.insert(bind_to.clone(), json_values);
            }
        }
    }

    // Build output for requested return variables only.
    let mut output: HashMap<String, Vec<sonic_rs::Value>> = HashMap::new();
    for var_name in return_vars {
        let vals = json_store.remove(var_name).unwrap_or_default();
        output.insert(var_name.clone(), vals);
    }

    Ok(output)
}

fn execute_traversal_step<'db, 'arena>(
    step: &LoweredStep,
    storage: &'db SparrowGraphStorage,
    arena: &'arena Bump,
    live_store: &mut HashMap<String, Vec<TraversalValue<'arena>>>,
    json_store: &mut HashMap<String, Vec<sonic_rs::Value>>,
) -> Result<(), RuntimeError>
where
    'db: 'arena,
{
    let txn = storage
        .graph_env
        .read_txn()
        .map_err(|e| RuntimeError::Execution(GraphError::StorageError(e.to_string())))?;

    let values: Vec<TraversalValue<'arena>> = if let Some(seed_name) = &step.seed_var {
        let seeds: Vec<TraversalValue<'arena>> =
            live_store.get(seed_name).cloned().unwrap_or_default();
        execute_query_chain_from_seed(
            &step.tool_args,
            storage,
            &txn,
            arena,
            seeds.into_iter(),
        )
        .map_err(RuntimeError::Execution)?
        .collect()
        .map_err(RuntimeError::Execution)?
    } else {
        execute_query_chain(&step.tool_args, storage, &txn, arena)
            .map_err(RuntimeError::Execution)?
            .collect()
            .map_err(RuntimeError::Execution)?
    };

    // Serialise immediately; keep live values for potential seeding later.
    let json_values: Vec<sonic_rs::Value> = values
        .iter()
        .map(|v| {
            sonic_rs::to_value(v).unwrap_or_else(|_| sonic_rs::Value::default())
        })
        .collect();

    live_store.insert(step.bind_to.clone(), values);
    json_store.insert(step.bind_to.clone(), json_values);

    Ok(())
}
