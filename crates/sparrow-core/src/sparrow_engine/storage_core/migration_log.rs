use crate::sparrow_engine::types::GraphError;
use heed3::{Database, RoTxn, RwTxn, types::Bytes};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MigrationRecord {
    pub applied_at: u64,
    pub checksum: u64,
    pub status: MigrationStatus,
    pub reversible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MigrationStatus {
    InProgress,
    Complete,
}

impl MigrationRecord {
    pub fn in_progress(checksum: u64) -> Self {
        Self {
            applied_at: 0,
            checksum,
            status: MigrationStatus::InProgress,
            reversible: false,
        }
    }

    pub fn complete(checksum: u64) -> Self {
        Self {
            applied_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            checksum,
            status: MigrationStatus::Complete,
            reversible: false,
        }
    }
}

pub fn read_record(
    txn: &RoTxn,
    db: &Database<Bytes, Bytes>,
    name: &str,
) -> Result<Option<MigrationRecord>, GraphError> {
    match db.get(txn, name.as_bytes())? {
        None => Ok(None),
        Some(bytes) => {
            let record: MigrationRecord = bincode::deserialize(bytes)
                .map_err(|e| GraphError::New(format!("failed to deserialize MigrationRecord: {e}")))?;
            Ok(Some(record))
        }
    }
}

pub fn write_record(
    txn: &mut RwTxn,
    db: &Database<Bytes, Bytes>,
    name: &str,
    record: &MigrationRecord,
) -> Result<(), GraphError> {
    let bytes = bincode::serialize(record)
        .map_err(|e| GraphError::New(format!("failed to serialize MigrationRecord: {e}")))?;
    db.put(txn, name.as_bytes(), &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_record_round_trip() {
        let record = MigrationRecord {
            applied_at: 1234567890,
            checksum: 0xdeadbeef,
            status: MigrationStatus::Complete,
            reversible: false,
        };
        let bytes = bincode::serialize(&record).unwrap();
        let decoded: MigrationRecord = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded, record);
    }

    #[test]
    fn in_progress_record_round_trip() {
        let record = MigrationRecord {
            applied_at: 0,
            checksum: 42,
            status: MigrationStatus::InProgress,
            reversible: false,
        };
        let bytes = bincode::serialize(&record).unwrap();
        let decoded: MigrationRecord = bincode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.status, MigrationStatus::InProgress);
    }

    #[test]
    fn in_progress_constructor() {
        let r = MigrationRecord::in_progress(0xabcd);
        assert_eq!(r.status, MigrationStatus::InProgress);
        assert_eq!(r.applied_at, 0);
        assert_eq!(r.checksum, 0xabcd);
    }

    #[test]
    fn complete_constructor_sets_timestamp() {
        let r = MigrationRecord::complete(0x1234);
        assert_eq!(r.status, MigrationStatus::Complete);
        assert!(r.applied_at > 0);
        assert_eq!(r.checksum, 0x1234);
    }
}
