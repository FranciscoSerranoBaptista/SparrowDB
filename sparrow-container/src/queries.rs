
// DEFAULT CODE
// use sparrow_db::sparrow_engine::traversal_core::config::Config;

// pub fn config() -> Option<Config> {
//     None
// }



use bumpalo::Bump;
use sparrow_macros::{handler, tool_call, mcp_handler, migration};
use sparrow_db::{
    sparrow_engine::{
        reranker::{
            RerankAdapter,
            fusion::{RRFReranker, MMRReranker, DistanceMethod},
        },
        storage_core::txn::{ReadTransaction, WriteTransaction},
        traversal_core::{
            RTxn,
            config::{Config, GraphConfig, VectorConfig},
            ops::{
                bm25::search_bm25::SearchBM25Adapter,
                g::G,
                in_::{in_::InAdapter, in_e::InEdgesAdapter, to_n::ToNAdapter, to_v::ToVAdapter},
                out::{
                    from_n::FromNAdapter, from_v::FromVAdapter, out::OutAdapter, out_e::OutEdgesAdapter,
                },
                source::{
                    add_e::AddEAdapter,
                    add_n::AddNAdapter,
                    e_from_id::EFromIdAdapter,
                    e_from_type::EFromTypeAdapter,
                    n_from_id::NFromIdAdapter,
                    n_from_index::NFromIndexAdapter,
                    n_from_type::NFromTypeAdapter,
                    v_from_id::VFromIdAdapter,
                    v_from_type::VFromTypeAdapter
                },
                util::{
                    dedup::DedupAdapter, drop::Drop, exist::Exist,
                    filter_ref::FilterRefAdapter, map::MapAdapter, paths::{PathAlgorithm, ShortestPathAdapter},
                    range::RangeAdapter, update::UpdateAdapter, order::OrderByAdapter,
                    aggregate::AggregateAdapter, group_by::GroupByAdapter, count::CountAdapter,
                },
                vectors::{
                    brute_force_search::BruteForceSearchVAdapter, insert::InsertVAdapter,
                    search::SearchVAdapter,
                },
            },
            traversal_value::TraversalValue,
        },
        types::{GraphError, SecondaryIndex},
        vector_core::vector::HVector,
    },
    sparrow_gateway::{
        embedding_providers::{EmbeddingModel, get_embedding_model},
        router::router::{HandlerInput, IoContFn},
        mcp::mcp::{MCPHandlerSubmission, MCPToolInput, MCPHandler}
    },
    node_matches, props, embed, embed_async,
    field_addition_from_old_field, field_type_cast, field_addition_from_value,
    protocol::{
        response::Response,
        value::{casting::{cast, CastType}, Value},
        format::Format,
    },
    utils::{
        id::{ID, uuid_str},
        items::{Edge, Node},
        properties::ImmutablePropertiesMap,
    },
};
use sonic_rs::{Deserialize, Serialize, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use chrono::{DateTime, Utc};

// Re-export scalar types for generated code
type I8 = i8;
type I16 = i16;
type I32 = i32;
type I64 = i64;
type U8 = u8;
type U16 = u16;
type U32 = u32;
type U64 = u64;
type U128 = u128;
type F32 = f32;
type F64 = f64;
    
pub fn config() -> Option<Config> {
return Some(Config {
vector_config: Some(VectorConfig {
m: Some(16),
ef_construction: Some(128),
ef_search: Some(768),
}),
graph_config: Some(GraphConfig {
secondary_indices: Some(vec![SecondaryIndex::Unique("person_id".to_string()), SecondaryIndex::Unique("name".to_string())]),
}),
db_max_size_gb: Some(10),
mcp: Some(true),
bm25: Some(true),
schema: Some(r#"{
  "schema": {
    "nodes": [
      {
        "name": "People",
        "properties": {
          "id": "ID",
          "person_id": "String",
          "last_name": "String",
          "age": "I32",
          "label": "String",
          "first_name": "String"
        }
      },
      {
        "name": "Company",
        "properties": {
          "label": "String",
          "name": "String",
          "id": "ID"
        }
      }
    ],
    "vectors": [],
    "edges": [
      {
        "name": "WorksAt",
        "from": "People",
        "to": "Company",
        "properties": {}
      }
    ]
  },
  "queries": [
    {
      "name": "dummy",
      "parameters": {},
      "returns": [
        "p"
      ]
    }
  ]
}"#.to_string()),
embedding_model: Some("text-embedding-ada-002".to_string()),
graphvis_node_label: None,
hql_schema_raw: Some(r#"QUERY dummy() =>
    p <- N<People>
RETURN p

N::People {
    UNIQUE INDEX person_id: String,
    first_name: String,
    last_name: String,
    age: I32
}
N::Company {
    UNIQUE INDEX name: String
}
E::WorksAt UNIQUE {
    From: People,
    To: Company,
    Properties: {}
}

"#.to_string()),
})
}
pub struct People {
    pub person_id: String,
    pub first_name: String,
    pub last_name: String,
    pub age: i32,
}

pub struct Company {
    pub name: String,
}

pub struct WorksAt {
    pub from: People,
    pub to: Company,
}


#[derive(Serialize, Default)]
pub struct DummyPReturnType<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub person_id: Option<&'a Value>,
    pub first_name: Option<&'a Value>,
    pub last_name: Option<&'a Value>,
    pub age: Option<&'a Value>,
}

#[handler]
pub fn dummy (input: HandlerInput) -> Result<Response, GraphError> {
let db = Arc::clone(&input.graph.storage);
let arena = Bump::new();
let txn = db.graph_env.read_txn().map_err(|e| GraphError::New(format!("Failed to start read transaction: {:?}", e)))?;
    let p = G::new(&db, &txn, &arena)
.n_from_type("People").collect::<Result<Vec<_>, _>>()?;
let response = json!({
    "p": p.iter().map(|p| DummyPReturnType {
        id: uuid_str(p.id(), &arena),
        label: p.label(),
        person_id: p.get_property("person_id"),
        first_name: p.get_property("first_name"),
        last_name: p.get_property("last_name"),
        age: p.get_property("age"),
    }).collect::<Vec<_>>()
});
txn.commit().map_err(|e| GraphError::New(format!("Failed to commit transaction: {:?}", e)))?;
Ok(input.request.out_fmt.create_response(&response))
}


