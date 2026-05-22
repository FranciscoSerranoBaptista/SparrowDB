use crate::sparrow_engine::types::GraphError;
use crate::sparrow_gateway::router::router::{Handler, HandlerInput, HandlerSubmission};
use crate::protocol;

// POST /migrate_status
// Returns the status of every compiled schema migration recorded in the migrations log.
// Each entry reflects the last run of that transition (InProgress or Complete).
//
// [
//   {"name":"User_v1_v2","status":"Complete","applied_at":1716000000,"checksum":12345,"reversible":false},
//   ...
// ]

fn migrate_status(input: HandlerInput) -> Result<protocol::Response, GraphError> {
    #[cfg(feature = "lmdb")]
    {
        use crate::sparrow_engine::storage_core::{
            migration_log::read_record,
            version_info::TransitionSubmission,
        };

        let db = &input.graph.storage;
        let txn = db.graph_env.read_txn()?;

        let mut entries = Vec::new();
        for submission in inventory::iter::<TransitionSubmission> {
            let t = &submission.0;
            let migration_name = format!(
                "{}_v{}_v{}",
                t.item_label, t.from_version, t.to_version
            );
            let record = read_record(&txn, &db.migrations_db, &migration_name)?;
            let status_str = match &record {
                Some(r) => match r.status {
                    crate::sparrow_engine::storage_core::migration_log::MigrationStatus::Complete => "Complete",
                    crate::sparrow_engine::storage_core::migration_log::MigrationStatus::InProgress => "InProgress",
                },
                None => "NotRun",
            };
            let applied_at = record.as_ref().map(|r| r.applied_at).unwrap_or(0);
            let checksum = record.as_ref().map(|r| r.checksum).unwrap_or(0);
            let reversible = record.as_ref().map(|r| r.reversible).unwrap_or(false);

            entries.push(format!(
                r#"{{"name":"{name}","status":"{status}","applied_at":{applied_at},"checksum":{checksum},"reversible":{reversible}}}"#,
                name = migration_name,
                status = status_str,
            ));
        }

        let body = format!("[{}]", entries.join(","));
        return Ok(protocol::Response {
            body: body.into_bytes(),
            fmt: Default::default(),
        });
    }

    #[cfg(not(feature = "lmdb"))]
    Err(GraphError::New(
        "migrate_status requires lmdb feature".to_string(),
    ))
}

#[cfg(feature = "dev-instance")]
inventory::submit! {
    HandlerSubmission(Handler::new("migrate_status", migrate_status, false))
}

// POST /migrate_list
// Returns all schema transitions compiled into the binary (from inventory).
//
// [
//   {"label":"User","from_version":1,"to_version":2,"reversible":false},
//   ...
// ]

fn migrate_list(_input: HandlerInput) -> Result<protocol::Response, GraphError> {
    use crate::sparrow_engine::storage_core::version_info::TransitionSubmission;

    let mut entries = Vec::new();
    for submission in inventory::iter::<TransitionSubmission> {
        let t = &submission.0;
        entries.push(format!(
            r#"{{"label":"{label}","from_version":{from_version},"to_version":{to_version},"reversible":{reversible}}}"#,
            label = t.item_label,
            from_version = t.from_version,
            to_version = t.to_version,
            reversible = t.reversible,
        ));
    }

    let body = format!("[{}]", entries.join(","));
    Ok(protocol::Response {
        body: body.into_bytes(),
        fmt: Default::default(),
    })
}

#[cfg(feature = "dev-instance")]
inventory::submit! {
    HandlerSubmission(Handler::new("migrate_list", migrate_list, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sparrow_engine::{
            storage_core::version_info::VersionInfo,
            traversal_core::{SparrowGraphEngine, SparrowGraphEngineOpts, config::Config},
        },
        sparrow_gateway::router::router::HandlerInput,
        protocol::{Format, request::Request, request::RequestType},
    };
    use axum::body::Bytes;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn setup_engine() -> (SparrowGraphEngine, TempDir) {
        let dir = TempDir::new().unwrap();
        let opts = SparrowGraphEngineOpts {
            path: dir.path().to_str().unwrap().to_string(),
            config: Config::default(),
            version_info: VersionInfo::default(),
        };
        (SparrowGraphEngine::new(opts).unwrap(), dir)
    }

    fn make_request(name: &str) -> Request {
        Request {
            name: name.to_string(),
            req_type: RequestType::Query,
            api_key: None,
            body: Bytes::new(),
            in_fmt: Format::Json,
            out_fmt: Format::Json,
            pre_computed_embedding: None,
        }
    }

    #[test]
    fn migrate_list_returns_json_array() {
        let (engine, _dir) = setup_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_request("migrate_list"),
        };
        let result = migrate_list(input);
        assert!(result.is_ok());
        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.starts_with('[') && body.ends_with(']'));
    }

    #[test]
    fn migrate_status_returns_json_array() {
        let (engine, _dir) = setup_engine();
        let input = HandlerInput {
            graph: Arc::new(engine),
            request: make_request("migrate_status"),
        };
        let result = migrate_status(input);
        assert!(result.is_ok());
        let body = String::from_utf8(result.unwrap().body).unwrap();
        assert!(body.starts_with('[') && body.ends_with(']'));
    }
}
