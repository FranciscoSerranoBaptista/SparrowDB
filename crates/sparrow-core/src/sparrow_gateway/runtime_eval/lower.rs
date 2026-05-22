use crate::{
    protocol::value::Value,
    sparrow_gateway::mcp::tools::{
        EdgeType as McpEdgeType, FilterProperties, FilterTraversal, Operator, ToolArgs,
    },
    sparrowc::parser::types::{
        BooleanOpType, ExpressionType, FieldAddition, FieldValue, FieldValueType,
        GraphStepType, IdType, ReturnType, Source, StartNode, StatementType, StepType, ValueType,
    },
};
use std::collections::HashMap;
use super::RuntimeError;

pub struct LoweredStep {
    /// None = start fresh from storage; Some("var") = seed from variable store
    pub seed_var: Option<String>,
    pub tool_args: Vec<ToolArgs>,
    pub bind_to: String,
}

pub enum MutationOp {
    AddNode {
        node_type: String,
        fields: HashMap<String, Value>,
    },
    AddEdge {
        edge_type: String,
        from_var: String,
        to_var: String,
        fields: HashMap<String, Value>,
    },
    DropNodes {
        seed_var: Option<String>,
        tool_args: Vec<ToolArgs>,
    },
    UpdateNodes {
        seed_var: Option<String>,
        tool_args: Vec<ToolArgs>,
        updates: HashMap<String, Value>,
    },
}

pub enum LoweredOp {
    Traversal(LoweredStep),
    Mutation { bind_to: String, op: MutationOp },
}

/// Lower a single-query Source into executable ops.
/// Returns (ops, return_var_names).
pub fn lower_query(
    source: &Source,
    params: &HashMap<String, Value>,
) -> Result<(Vec<LoweredOp>, Vec<String>), RuntimeError> {
    // The schema_raw passed to parse_and_validate may itself contain queries (e.g. the
    // compiled dummy/stub queries baked into the binary).  Our submitted query is always
    // appended *after* those, so we must take the *last* query in the parsed source.
    let query = source
        .queries
        .last()
        .ok_or_else(|| RuntimeError::Lowering("source has no queries".to_string()))?;

    let mut ops = Vec::new();
    for stmt in &query.statements {
        match &stmt.statement {
            StatementType::Assignment(assign) => {
                let op = lower_expr(&assign.value.expr, params, &assign.variable)?;
                ops.push(op);
            }
            StatementType::Drop(expr) => {
                match &expr.expr {
                    ExpressionType::Traversal(trav) => {
                        let (seed_var, mut all_args) = lower_start(&trav.start, params)?;
                        for step in &trav.steps {
                            all_args.extend(lower_step(&step.step, params)?);
                        }
                        if seed_var.is_none() && all_args.is_empty() {
                            return Err(RuntimeError::Lowering(
                                "DROP requires a non-empty traversal (cannot drop without a target)".to_string(),
                            ));
                        }
                        ops.push(LoweredOp::Mutation {
                            bind_to: "_drop_result".to_string(),
                            op: MutationOp::DropNodes {
                                seed_var,
                                tool_args: all_args,
                            },
                        });
                    }
                    _ => {
                        return Err(RuntimeError::Unsupported(
                            "DROP with non-traversal expression".to_string(),
                        ));
                    }
                }
            }
            StatementType::Expression(_) | StatementType::ForLoop(_) => {
                return Err(RuntimeError::Unsupported(
                    "Expression/ForLoop statements not yet supported".to_string(),
                ));
            }
        }
    }

    let return_vars: Vec<String> = query
        .return_values
        .iter()
        .filter_map(|rv| match rv {
            ReturnType::Expression(e) => match &e.expr {
                ExpressionType::Identifier(name) => Some(name.clone()),
                _ => None,
            },
            ReturnType::Empty => None,
            ReturnType::Array(_) | ReturnType::Object(_) => None,
        })
        .collect();

    Ok((ops, return_vars))
}

fn lower_expr(
    expr: &ExpressionType,
    params: &HashMap<String, Value>,
    bind_to: &str,
) -> Result<LoweredOp, RuntimeError> {
    match expr {
        ExpressionType::Traversal(traversal) => lower_traversal(traversal, params, bind_to),
        ExpressionType::AddNode(an) => {
            let node_type = an
                .node_type
                .as_deref()
                .ok_or_else(|| RuntimeError::Lowering("AddNode missing type".to_string()))?
                .to_string();
            let fields = lower_fields_map(an.fields.as_ref(), params)?;
            Ok(LoweredOp::Mutation {
                bind_to: bind_to.to_string(),
                op: MutationOp::AddNode { node_type, fields },
            })
        }
        ExpressionType::AddEdge(ae) => {
            let edge_type = ae
                .edge_type
                .as_deref()
                .ok_or_else(|| RuntimeError::Lowering("AddEdge missing type".to_string()))?
                .to_string();
            let from_var = match ae.connection.from_id.as_ref() {
                Some(IdType::Identifier { value, .. }) => value.clone(),
                _ => {
                    return Err(RuntimeError::Lowering(
                        "AddEdge From must be a variable identifier".to_string(),
                    ))
                }
            };
            let to_var = match ae.connection.to_id.as_ref() {
                Some(IdType::Identifier { value, .. }) => value.clone(),
                _ => {
                    return Err(RuntimeError::Lowering(
                        "AddEdge To must be a variable identifier".to_string(),
                    ))
                }
            };
            let fields = lower_fields_map(ae.fields.as_ref(), params)?;
            Ok(LoweredOp::Mutation {
                bind_to: bind_to.to_string(),
                op: MutationOp::AddEdge {
                    edge_type,
                    from_var,
                    to_var,
                    fields,
                },
            })
        }
        _ => Err(RuntimeError::Unsupported(format!(
            "expression type not supported: {expr:?}"
        ))),
    }
}

fn lower_traversal(
    traversal: &crate::sparrowc::parser::types::Traversal,
    params: &HashMap<String, Value>,
    bind_to: &str,
) -> Result<LoweredOp, RuntimeError> {
    // Check if the last step is Update — if so, this becomes an UpdateNodes mutation.
    let (steps_to_lower, update_fields) = if let Some(last) = traversal.steps.last() {
        if let StepType::Update(u) = &last.step {
            (&traversal.steps[..traversal.steps.len() - 1], Some(&u.fields))
        } else {
            (&traversal.steps[..], None)
        }
    } else {
        (&traversal.steps[..], None)
    };

    let (seed_var, mut args) = lower_start(&traversal.start, params)?;
    for step in steps_to_lower {
        args.extend(lower_step(&step.step, params)?);
    }

    if let Some(fields) = update_fields {
        if seed_var.is_none() && args.is_empty() {
            return Err(RuntimeError::Lowering(
                "UPDATE requires a non-empty traversal (cannot update without a target)".to_string(),
            ));
        }
        let updates = lower_field_additions(fields, params)?;
        Ok(LoweredOp::Mutation {
            bind_to: bind_to.to_string(),
            op: MutationOp::UpdateNodes {
                seed_var,
                tool_args: args,
                updates,
            },
        })
    } else {
        Ok(LoweredOp::Traversal(LoweredStep {
            seed_var,
            tool_args: args,
            bind_to: bind_to.to_string(),
        }))
    }
}

fn lower_start(
    start: &StartNode,
    params: &HashMap<String, Value>,
) -> Result<(Option<String>, Vec<ToolArgs>), RuntimeError> {
    match start {
        StartNode::Node { node_type, ids } => {
            let mut args = vec![ToolArgs::NFromType {
                node_type: node_type.clone(),
            }];
            if let Some(id_list) = ids {
                let filter = ids_to_filter(id_list, params)?;
                args.push(ToolArgs::FilterItems { filter });
            }
            Ok((None, args))
        }
        StartNode::Edge { edge_type, ids } => {
            let mut args = vec![ToolArgs::EFromType {
                edge_type: edge_type.clone(),
            }];
            if let Some(id_list) = ids {
                let filter = ids_to_filter(id_list, params)?;
                args.push(ToolArgs::FilterItems { filter });
            }
            Ok((None, args))
        }
        StartNode::Identifier(name) => Ok((Some(name.clone()), vec![])),
        StartNode::Anonymous => Ok((None, vec![])),
        StartNode::Vector { .. }
        | StartNode::SearchVector(_)
        | StartNode::SearchNodeVector(_) => Err(RuntimeError::Unsupported(
            "SearchVector/Vector/SearchNodeVector start nodes not yet supported".to_string(),
        )),
    }
}

fn ids_to_filter(
    ids: &[IdType],
    params: &HashMap<String, Value>,
) -> Result<FilterTraversal, RuntimeError> {
    let mut props = Vec::new();
    for id in ids {
        match id {
            IdType::ByIndex { index, value, .. } => {
                let field = index.to_string();
                let val = resolve_value_type(value, params)?;
                props.push(FilterProperties {
                    key: field,
                    value: val,
                    operator: Some(Operator::Eq),
                });
            }
            IdType::Literal { value, .. } => {
                return Err(RuntimeError::Unsupported(format!(
                    "bare literal id: {value}"
                )));
            }
            IdType::Identifier { value, .. } => {
                return Err(RuntimeError::Unsupported(format!(
                    "bare identifier id: {value}"
                )));
            }
        }
    }
    Ok(FilterTraversal {
        properties: Some(vec![props]),
        filter_traversals: None,
    })
}

fn resolve_value_type(
    vt: &ValueType,
    params: &HashMap<String, Value>,
) -> Result<Value, RuntimeError> {
    match vt {
        ValueType::Literal { value, .. } => Ok(value.clone()),
        ValueType::Identifier { value: name, .. } => params
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::Lowering(format!("parameter '{name}' not provided"))),
        ValueType::Object { .. } => {
            Err(RuntimeError::Unsupported("object value types".to_string()))
        }
    }
}

fn lower_step(
    step: &StepType,
    params: &HashMap<String, Value>,
) -> Result<Vec<ToolArgs>, RuntimeError> {
    match step {
        StepType::Node(gs) | StepType::Edge(gs) => match &gs.step {
            GraphStepType::Out(label) => Ok(vec![ToolArgs::OutStep {
                edge_label: label.clone(),
                edge_type: McpEdgeType::Node,
                filter: None,
            }]),
            GraphStepType::In(label) => Ok(vec![ToolArgs::InStep {
                edge_label: label.clone(),
                edge_type: McpEdgeType::Node,
                filter: None,
            }]),
            GraphStepType::OutE(label) => Ok(vec![ToolArgs::OutEStep {
                edge_label: label.clone(),
                filter: None,
            }]),
            GraphStepType::InE(label) => Ok(vec![ToolArgs::InEStep {
                edge_label: label.clone(),
                filter: None,
            }]),
            other => Err(RuntimeError::Unsupported(format!(
                "graph step variant: {other:?}"
            ))),
        },
        StepType::Where(where_expr) => lower_where_expr(where_expr, params),
        other => Err(RuntimeError::Unsupported(format!("step type: {other:?}"))),
    }
}

fn lower_where_expr(
    expr: &crate::sparrowc::parser::types::Expression,
    params: &HashMap<String, Value>,
) -> Result<Vec<ToolArgs>, RuntimeError> {
    match &expr.expr {
        ExpressionType::Traversal(trav) => {
            let mut field_name: Option<String> = None;
            let mut operator: Option<Operator> = None;
            let mut rhs: Option<Value> = None;

            for step in &trav.steps {
                match &step.step {
                    StepType::Object(obj) => {
                        // Object.fields is Vec<FieldAddition>, each has .key: String
                        if obj.fields.len() > 1 {
                            return Err(RuntimeError::Unsupported(
                                "WHERE with multiple fields not yet supported".to_string(),
                            ));
                        }
                        if let Some(first) = obj.fields.first() {
                            field_name = Some(first.key.clone());
                        }
                    }
                    StepType::BooleanOperation(bool_op) => {
                        let (op, rhs_expr) = match &bool_op.op {
                            BooleanOpType::Equal(e) => (Operator::Eq, e.as_ref()),
                            BooleanOpType::NotEqual(e) => (Operator::Neq, e.as_ref()),
                            BooleanOpType::GreaterThan(e) => (Operator::Gt, e.as_ref()),
                            BooleanOpType::GreaterThanOrEqual(e) => (Operator::Gte, e.as_ref()),
                            BooleanOpType::LessThan(e) => (Operator::Lt, e.as_ref()),
                            BooleanOpType::LessThanOrEqual(e) => (Operator::Lte, e.as_ref()),
                            other => {
                                return Err(RuntimeError::Unsupported(format!(
                                    "bool op: {other:?}"
                                )))
                            }
                        };
                        operator = Some(op);
                        rhs = Some(resolve_rhs_expr(rhs_expr, params)?);
                    }
                    _ => {}
                }
            }

            let field = field_name.ok_or_else(|| {
                RuntimeError::Lowering("WHERE missing field".to_string())
            })?;
            let op = operator.ok_or_else(|| {
                RuntimeError::Lowering("WHERE missing operator".to_string())
            })?;
            let val = rhs.ok_or_else(|| {
                RuntimeError::Lowering("WHERE missing rhs value".to_string())
            })?;

            Ok(vec![ToolArgs::FilterItems {
                filter: FilterTraversal {
                    properties: Some(vec![vec![FilterProperties {
                        key: field,
                        value: val,
                        operator: Some(op),
                    }]]),
                    filter_traversals: None,
                },
            }])
        }
        _ => Err(RuntimeError::Unsupported(
            "non-traversal WHERE expression".to_string(),
        )),
    }
}

fn resolve_rhs_expr(
    expr: &crate::sparrowc::parser::types::Expression,
    params: &HashMap<String, Value>,
) -> Result<Value, RuntimeError> {
    match &expr.expr {
        ExpressionType::Identifier(name) => params
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::Lowering(format!("parameter '{name}' not provided"))),
        ExpressionType::StringLiteral(s) => Ok(Value::String(s.clone())),
        ExpressionType::IntegerLiteral(i) => Ok(Value::I32(*i)),
        ExpressionType::FloatLiteral(f) => Ok(Value::F64(*f)),
        ExpressionType::BooleanLiteral(b) => Ok(Value::Boolean(*b)),
        _ => Err(RuntimeError::Unsupported(
            "complex rhs expression".to_string(),
        )),
    }
}

fn lower_fields_map(
    fields: Option<&HashMap<String, ValueType>>,
    params: &HashMap<String, Value>,
) -> Result<HashMap<String, Value>, RuntimeError> {
    let mut out = HashMap::new();
    if let Some(f) = fields {
        for (k, vt) in f {
            let v = resolve_value_type(vt, params)?;
            out.insert(k.clone(), v);
        }
    }
    Ok(out)
}

fn lower_field_additions(
    fields: &[FieldAddition],
    params: &HashMap<String, Value>,
) -> Result<HashMap<String, Value>, RuntimeError> {
    let mut out = HashMap::new();
    for fa in fields {
        let v = resolve_field_value(&fa.value, params)?;
        out.insert(fa.key.clone(), v);
    }
    Ok(out)
}

fn resolve_field_value(
    fv: &FieldValue,
    params: &HashMap<String, Value>,
) -> Result<Value, RuntimeError> {
    match &fv.value {
        FieldValueType::Literal(v) => Ok(v.clone()),
        FieldValueType::Identifier(name) => params
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::Lowering(format!("parameter '{name}' not provided"))),
        FieldValueType::Expression(expr) => resolve_rhs_expr(expr, params),
        _ => Err(RuntimeError::Unsupported(
            "complex field value type".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparrow_gateway::runtime_eval::parse::parse_and_validate;

    const SCHEMA: &str = r#"
N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    age: I32
}
E::Knows UNIQUE {
    From: People,
    To: People,
    Properties: {}
}
"#;

    fn do_lower(query: &str, params: &[(&str, Value)]) -> (Vec<LoweredOp>, Vec<String>) {
        let param_map: HashMap<String, Value> = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let source = parse_and_validate(SCHEMA, query).unwrap();
        lower_query(&source, &param_map).unwrap()
    }

    #[test]
    fn test_lower_n_scan() {
        let (ops, ret) = do_lower(
            "QUERY getAll() =>\n    p <- N<People>\nRETURN p",
            &[],
        );
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            ops[0],
            LoweredOp::Traversal(LoweredStep {
                tool_args: ref args,
                ..
            }) if matches!(args[0], ToolArgs::NFromType { .. })
        ));
        assert_eq!(ret, vec!["p"]);
    }

    #[test]
    fn test_lower_n_with_index_filter() {
        let (ops, _) = do_lower(
            "QUERY get(pid: String) =>\n    p <- N<People>({person_id: pid})\nRETURN p",
            &[("pid", Value::String("alice".to_string()))],
        );
        assert!(matches!(&ops[0], LoweredOp::Traversal(step) if step.tool_args.len() == 2));
        assert!(matches!(
            &ops[0],
            LoweredOp::Traversal(step) if matches!(step.tool_args[0], ToolArgs::NFromType { .. })
        ));
        assert!(matches!(
            &ops[0],
            LoweredOp::Traversal(step) if matches!(step.tool_args[1], ToolArgs::FilterItems { .. })
        ));
    }

    #[test]
    fn test_lower_traversal_chain() {
        let (ops, ret) = do_lower(
            r#"
QUERY getFriends(pid: String) =>
    p <- N<People>({person_id: pid})
    friends <- p::Out<Knows>
RETURN friends
"#,
            &[("pid", Value::String("alice".to_string()))],
        );
        assert_eq!(ops.len(), 2);
        assert!(
            matches!(&ops[1], LoweredOp::Traversal(step) if step.seed_var.as_deref() == Some("p"))
        );
        assert!(
            matches!(&ops[1], LoweredOp::Traversal(step) if matches!(step.tool_args[0], ToolArgs::OutStep { .. }))
        );
        assert_eq!(ret, vec!["friends"]);
    }

    // ---- Mutation tests ----

    const MUTATION_SCHEMA: &str = r#"
N::Item { UNIQUE INDEX item_id: String, item_label: String }
E::Links UNIQUE { From: Item, To: Item, Properties: {} }
"#;

    #[test]
    fn test_lower_add_node() {
        let query = r#"
QUERY create(item_id: String, item_label: String) =>
    item <- AddN<Item>({item_id: item_id, item_label: item_label})
RETURN item
"#;
        let params: HashMap<String, Value> = [
            ("item_id".to_string(), Value::String("x1".to_string())),
            ("item_label".to_string(), Value::String("hello".to_string())),
        ]
        .into();
        let source = parse_and_validate(MUTATION_SCHEMA, query).unwrap();
        let (ops, ret) = lower_query(&source, &params).unwrap();
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            ops[0],
            LoweredOp::Mutation {
                op: MutationOp::AddNode { .. },
                ..
            }
        ));
        assert_eq!(ret, vec!["item"]);
    }

    #[test]
    fn test_lower_add_edge() {
        let query = r#"
QUERY link(a_id: String, b_id: String) =>
    a <- N<Item>({item_id: a_id})
    b <- N<Item>({item_id: b_id})
    edge <- AddE<Links>()::From(a)::To(b)
RETURN edge
"#;
        let params: HashMap<String, Value> = [
            ("a_id".to_string(), Value::String("a1".to_string())),
            ("b_id".to_string(), Value::String("b1".to_string())),
        ]
        .into();
        let source = parse_and_validate(MUTATION_SCHEMA, query).unwrap();
        let (ops, ret) = lower_query(&source, &params).unwrap();
        assert_eq!(ops.len(), 3); // two N lookups + one AddEdge mutation
        assert!(matches!(
            &ops[2],
            LoweredOp::Mutation {
                op: MutationOp::AddEdge { from_var, to_var, .. },
                ..
            } if from_var == "a" && to_var == "b"
        ));
        assert_eq!(ret, vec!["edge"]);
    }

    #[test]
    fn test_lower_drop() {
        let query = r#"
QUERY del(item_id: String) =>
    DROP N<Item>({item_id: item_id})
RETURN NONE
"#;
        let params: HashMap<String, Value> = [("item_id".to_string(), Value::String("x1".to_string()))]
            .into();
        let source = parse_and_validate(MUTATION_SCHEMA, query).unwrap();
        let (ops, _) = lower_query(&source, &params).unwrap();
        assert!(ops
            .iter()
            .any(|op| matches!(op, LoweredOp::Mutation { op: MutationOp::DropNodes { .. }, .. })));
    }

    #[test]
    fn test_lower_update() {
        let query = r#"
QUERY updateLabel(item_id: String, new_label: String) =>
    item <- N<Item>({item_id: item_id})::UPDATE({item_label: new_label})
RETURN item
"#;
        let params: HashMap<String, Value> = [
            ("item_id".to_string(), Value::String("x1".to_string())),
            ("new_label".to_string(), Value::String("updated".to_string())),
        ]
        .into();
        let source = parse_and_validate(MUTATION_SCHEMA, query).unwrap();
        let (ops, ret) = lower_query(&source, &params).unwrap();
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            ops[0],
            LoweredOp::Mutation {
                op: MutationOp::UpdateNodes { .. },
                ..
            }
        ));
        assert_eq!(ret, vec!["item"]);
    }
}
