You have hit on the exact reason the original developers designed HelixQL the way they did. You are looking at a classic database engineering trade-off.

To answer your question directly: **Standard Cypher (like in Neo4j) *doesn't* guarantee types.** It is dynamically typed and "schema-optional." In Neo4j, a `Person` node might have an `age` that is an integer, while another `Person` node has an `age` that is a string, and a third `Person` has no `age` at all. The errors only happen at runtime.

For Rust developers, that is terrifying. The entire value proposition of SparrowDB is **AOT (Ahead-of-Time) compilation**: taking a query, validating it against a strict schema, and generating memory-safe, strongly-typed Rust structs for the output.

If you adopt a Cypher-like syntax, you do not have to adopt Cypher’s dynamic typing. You can build **Statically Typed Cypher**. Here is how you separate the *syntax* (what the user writes) from the *semantics* (how the compiler guarantees types).

### How to build a Statically Typed Cypher Compiler

Right now, in `sparrow-db/src/sparrowc/analyzer/methods/infer_expr_type.rs`, the SparrowDB compiler does type inference. If you switch the parser to accept Cypher, you just feed the Cypher AST into the exact same type-inference engine.

Here is how it works under the hood:

#### 1. The Schema acts as the Type Catalog
You keep a strict schema definition. The compiler knows exactly what every node and edge looks like.
```rust
// The Compiler knows this:
Node::Person { name: String, age: U32 }
Edge::Knows  { since: Date }
```

#### 2. Pattern Matching = Type Inference
When the user writes a `MATCH` clause, the compiler looks at the labels and assigns static types to the variables in the AST scope.

```cypher
MATCH (p:Person)-[k:Knows]->(f:Person)
```
The compiler's Semantic Analyzer steps in:
1. `p` has label `Person` $\rightarrow$ `p` is of type `Type::Node("Person")`.
2. `k` has label `Knows` $\rightarrow$ `k` is of type `Type::Edge("Knows")`.
3. Validates that `Knows` is legally allowed to connect `Person` to `Person` (catching schema violations at compile time).

#### 3. Compile-Time Type Checking in `WHERE`
Because the compiler knows `p` is a `Person`, it knows `p.age` is a `U32`.

```cypher
WHERE p.age > "eighteen"
```
The Semantic Analyzer evaluates the AST:
1. LHS: `p.age` $\rightarrow$ Resolves to `U32`.
2. RHS: `"eighteen"` $\rightarrow$ Resolves to `String`.
3. Operator: `>` $\rightarrow$ Requires LHS and RHS to be comparable numeric types.
4. **Result:** The compiler immediately throws a rustc-style error: `TypeError: Cannot compare U32 with String`, *before the database even runs the query*.

#### 4. Strongly-Typed Code Generation
Because the compiler knows the exact types of what is being returned, it can generate the exact same Rust structs SparrowDB currently generates.

```cypher
RETURN p.name, k.since
```
The compiler looks at the return AST, sees `String` and `Date`, and generates:
```rust
#[derive(Serialize)]
pub struct QueryReturnType {
    pub name: String,
    pub since: DateTime<Utc>,
}
```

### Industry Context: The Move to Strict Graphs

You are not alone in wanting strict types in a graph database. The industry recognized that dynamic graphs are a nightmare for enterprise software.

In April 2024, the ISO officially published **GQL (Graph Query Language)** (ISO/IEC 39075:2024). It is the first new ISO database language since SQL in 1987.
One of the biggest additions in ISO GQL over openCypher is the introduction of **strict Graph Types and Catalogs**.

If you design your query language, you can look at **EdgeDB (EdgeQL)** for inspiration. EdgeDB is built on top of Postgres, but they completely replaced SQL with a strongly-typed graph-relational query language that is beloved by developers because it guarantees types at compile time (and generates native TypeScript/Rust types for the client).

### Conclusion for your Fork

You don't need to throw away SparrowDB's incredible AOT Rust compilation. The `sparrowc` analyzer is actually quite robust at type-checking.

The problem is simply that the *frontend syntax* (HelixQL) was designed to mimic Rust's internal syntax (`::`, `<T>`) because it was easy to map to the backend code generator.

If you write a parser that accepts a declarative, Cypher-like syntax (or a clean EdgeDB-like fluent syntax), and pipe that AST into the existing `Ctx` analyzer in `sparrowc/analyzer/mod.rs`, you get the holy grail: **The ergonomics of Cypher, with the memory and type safety of Rust.**

+++

To achieve the "Holy Grail" (Cypher/EdgeDB-like ergonomics on the frontend, with Rust’s strict type-safety and AOT compilation on the backend), we don't need to touch the `Ctx` semantic analyzer at all.

Instead, we build a **Compiler Frontend (Parser + Lowering Phase)**.

We will use the `chumsky` crate to parse a clean, declarative syntax. Then, we "lower" (translate) that declarative syntax directly into SparrowDB's existing imperative AST (`Query`, `Statement`, `Traversal`, etc.). Because the AST matches what `pest` used to output, the `Ctx` analyzer in `sparrowc/analyzer/mod.rs` will accept it, type-check it, and compile it to Rust as if nothing changed!

Here is a working blueprint of how to replace Pest with a modern Chumsky parser that compiles Cypher-like syntax into the existing SparrowDB AST.

### 1. The Target Syntax
Instead of writing this:
```rust
// Old HelixQL (Imperative, leaky abstractions)
QUERY getFriends(id: ID, minAge: U32) =>
    person <- N<Person>(id)
    friends <- person::Out<Knows>::WHERE(_::{age}::GT(minAge))
    RETURN friends
```

The user will write this:
```cypher
// New Declarative Syntax
query getFriends(id: ID, minAge: U32) {
    match (person:Person {id: id})-[:Knows]->(friends:Person)
    where friends.age > minAge
    return friends
}
```

### 2. The Implementation (Chumsky + Lowering)

Add `chumsky = "0.9"` to `sparrow-db/Cargo.toml`. Create a new file `sparrowc/parser/cypher.rs`.

```rust
use chumsky::prelude::*;
use crate::sparrowc::parser::types::*;
use crate::sparrowc::parser::location::{Loc, Span};

/// 1. INTERMEDIATE CYPHER AST
/// We parse into this clean AST first, then lower it to Sparrow's AST.
#[derive(Debug)]
pub struct CypherMatch {
    pub source_var: String,
    pub source_label: String,
    pub source_id: Option<String>,
    pub edge_label: String,
    pub target_var: String,
    pub target_label: String,
}

#[derive(Debug)]
pub struct CypherWhere {
    pub target_var: String,
    pub property: String,
    pub operator: String,
    pub value: String, // Can be literal or param
}

/// 2. THE CHUMSKY PARSER
/// A robust, error-recovering parser combinator
pub fn cypher_parser() -> impl Parser<char, Query, Error = Simple<char>> {
    // Parse: query getFriends(id: ID, minAge: U32)
    let ident = text::ident().padded();

    let param = ident
        .then_ignore(just(':').padded())
        .then(ident)
        .map(|(name, ty)| Parameter {
            name: (Loc::empty(), name),
            param_type: (Loc::empty(), parse_field_type(&ty)),
            is_optional: false,
            loc: Loc::empty(),
        });

    let params = param
        .separated_by(just(','))
        .delimited_by(just('('), just(')'))
        .padded();

    let query_decl = text::keyword("query")
        .ignore_then(ident)
        .then(params);

    // Parse: match (person:Person {id: id})-[:Knows]->(friends:Person)
    let match_clause = text::keyword("match").padded().ignore_then(
        just('(').ignore_then(ident)
        .then_ignore(just(':'))
        .then(ident)
        .then(just('{').ignore_then(text::keyword("id").padded().ignore_then(just(':')).ignore_then(ident)).then_ignore(just('}')).or_not())
        .then_ignore(just(')'))
        .then_ignore(just("-[:"))
        .then(ident)
        .then_ignore(just("]->("))
        .then(ident)
        .then_ignore(just(':'))
        .then(ident)
        .then_ignore(just(')'))
    ).map(|((((((source_var, source_label), source_id), edge_label), target_var), target_label))| {
        CypherMatch { source_var, source_label, source_id, edge_label, target_var, target_label }
    });

    // Parse: where friends.age > minAge
    let where_clause = text::keyword("where").padded().ignore_then(
        ident
        .then_ignore(just('.'))
        .then(ident)
        .then(just('>').to(">").or(just('<').to("<")).or(just("==").to("==")).padded())
        .then(ident)
    ).map(|(((target_var, property), operator), value)| {
        CypherWhere { target_var, property, operator: operator.to_string(), value }
    });

    // Parse: return friends
    let return_clause = text::keyword("return").padded()
        .ignore_then(ident)
        .map(|ret_var| {
            ReturnType::Expression(Expression {
                loc: Loc::empty(),
                expr: ExpressionType::Identifier(ret_var),
            })
        });

    // Combine into a full query
    query_decl
        .then(
            match_clause
                .then(where_clause.or_not())
                .then(return_clause)
                .delimited_by(just('{').padded(), just('}').padded())
        )
        .map(|((name, params), ((match_stmt, where_stmt), ret_stmt))| {
            // 3. THE LOWERING PHASE (Cypher AST -> Sparrow AST)
            lower_to_sparrow_ast(name, params, match_stmt, where_stmt, ret_stmt)
        })
}

/// 3. THE LOWERING ENGINE
/// This converts the declarative Cypher pattern into SparrowDB's imperative Steps
fn lower_to_sparrow_ast(
    name: String,
    parameters: Vec<Parameter>,
    m: CypherMatch,
    w: Option<CypherWhere>,
    ret: ReturnType
) -> Query {
    let mut statements = Vec::new();

    // Step 1: Assign the source node. (e.g., person <- N<Person>(id))
    let source_ids = m.source_id.map(|id| vec![IdType::Identifier { value: id, loc: Loc::empty() }]);
    let source_traversal = Traversal {
        start: StartNode::Node { node_type: m.source_label, ids: source_ids },
        steps: vec![],
        loc: Loc::empty(),
    };
    statements.push(Statement {
        loc: Loc::empty(),
        statement: StatementType::Assignment(Assignment {
            variable: m.source_var.clone(),
            value: Expression { loc: Loc::empty(), expr: ExpressionType::Traversal(Box::new(source_traversal)) },
            loc: Loc::empty(),
        }),
    });

    // Step 2: Traverse the edge. (e.g., friends <- person::Out<Knows>)
    let mut target_steps = vec![Step {
        loc: Loc::empty(),
        step: StepType::Node(GraphStep {
            loc: Loc::empty(),
            step: GraphStepType::Out(m.edge_label),
        }),
    }];

    // Step 3: Lower the WHERE clause into a Sparrow anonymous traversal (e.g., _::{age}::GT(minAge))
    if let Some(wh) = w {
        let op_type = match wh.operator.as_str() {
            ">" => BooleanOpType::GreaterThan(Box::new(Expression {
                loc: Loc::empty(),
                expr: ExpressionType::Identifier(wh.value),
            })),
            _ => unimplemented!("Other operators omitted for brevity"),
        };

        // Create: _::{property}::GT(value)
        let where_traversal = Traversal {
            start: StartNode::Anonymous,
            steps: vec![
                Step {
                    loc: Loc::empty(),
                    step: StepType::Object(Object {
                        loc: Loc::empty(),
                        should_spread: false,
                        fields: vec![FieldAddition {
                            key: wh.property,
                            value: FieldValue { loc: Loc::empty(), value: FieldValueType::Empty },
                            loc: Loc::empty(),
                        }],
                    }),
                },
                Step {
                    loc: Loc::empty(),
                    step: StepType::BooleanOperation(BooleanOp { loc: Loc::empty(), op: op_type }),
                },
            ],
            loc: Loc::empty(),
        };

        target_steps.push(Step {
            loc: Loc::empty(),
            step: StepType::Where(Box::new(Expression {
                loc: Loc::empty(),
                expr: ExpressionType::Traversal(Box::new(where_traversal)),
            })),
        });
    }

    // Assign the target node.
    let target_traversal = Traversal {
        start: StartNode::Identifier(m.source_var),
        steps: target_steps,
        loc: Loc::empty(),
    };
    statements.push(Statement {
        loc: Loc::empty(),
        statement: StatementType::Assignment(Assignment {
            variable: m.target_var,
            value: Expression { loc: Loc::empty(), expr: ExpressionType::Traversal(Box::new(target_traversal)) },
            loc: Loc::empty(),
        }),
    });

    // Return the perfectly formatted Sparrow AST!
    Query {
        original_query: "".to_string(),
        built_in_macro: None,
        name,
        parameters,
        statements,
        return_values: vec![ret],
        loc: Loc::empty(),
    }
}

// Helper to map string types to Sparrow FieldTypes
fn parse_field_type(ty: &str) -> FieldType {
    match ty {
        "ID" => FieldType::Uuid,
        "String" => FieldType::String,
        "U32" => FieldType::U32,
        _ => FieldType::String, // Fallback for brevity
    }
}
```

### Why this is the "Holy Grail"

1. **You keep `sparrowc/analyzer/mod.rs` entirely untouched.**
   Look closely at the `lower_to_sparrow_ast` function. We literally programmatically constructed the exact AST that the old Pest parser was building. When this `Query` object is handed to `ctx.check_queries()`, the existing semantic analyzer will do all the heavy lifting:
   * It will verify `person` is of type `Person`.
   * It will verify `Person` has an outbound `Knows` edge.
   * It will verify the `age` property exists on the `friends` node.
   * It will verify `minAge` is a `U32` and `age` is a `U32`.

2. **You keep `sparrowc/generator/queries.rs` untouched.**
   Because the AST hasn't changed, the Rust code generator will output the exact same hyper-optimized LMDB zero-copy graph traversal code it always has.

3. **Incredible Error Messages (`ariadne`)**
   Notice how everything in SparrowDB's AST takes a `loc: Loc`? `Chumsky` natively outputs character spans `(usize, usize)`. As you parse the Cypher syntax, you map those spans directly into the `Loc` structs. When a user writes `WHERE friends.ageee > 18`, your existing analyzer will throw `E202: property 'ageee' does not exist on Person`, and because you mapped the Chumsky spans to `Loc`, it will perfectly underline `ageee` in the terminal using `ariadne`.

### Next Steps to Productionize

To integrate this:
1. Swap `chumsky` for `pest` in `sparrowc/parser/mod.rs`.
2. Delete the `grammar.pest` file and the `*_parse_methods.rs` files.
3. Build out the `cypher_parser()` combinators to handle the full breadth of SparrowDB operations (like `AddN`, `SearchVector`, `GroupBy`, etc.).

For Vector Search, you can introduce a beautiful, native syntax extension like this:
```cypher
query searchSimilarDocs(queryText: String) {
    // Chumsky parses this directly into `StartNode::SearchVector`!
    match (doc:Document) by vector.similarity(doc.embedding, embed(queryText))
    return doc.title, doc.content
}
```

By bridging a modern declarative syntax into the existing semantic analyzer, you fix the glaring UX problem of HelixQL while retaining 100% of the Rust safety and graph traversal performance the database was built for.
