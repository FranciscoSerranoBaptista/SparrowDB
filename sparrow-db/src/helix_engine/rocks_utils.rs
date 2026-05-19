use rocksdb::{BoundColumnFamily, DBRawIteratorWithThreadMode, MultiThreaded, ReadOptions, TransactionDB};
use std::sync::Arc;

pub trait RocksUtils {
    fn raw_prefix_iter<'a>(
        &'a self,
        cf: &Arc<BoundColumnFamily<'a>>,
        prefix: &[u8],
    ) -> DBRawIteratorWithThreadMode<'a, rocksdb::Transaction<'a, TransactionDB<MultiThreaded>>>;
}

impl<'db> RocksUtils for rocksdb::Transaction<'db, TransactionDB<MultiThreaded>> {
    fn raw_prefix_iter<'a>(
        &'a self,
        cf: &Arc<BoundColumnFamily<'a>>,
        prefix: &[u8],
    ) -> DBRawIteratorWithThreadMode<'a, rocksdb::Transaction<'a, TransactionDB<MultiThreaded>>> {
        let mut opts = ReadOptions::default();
        opts.set_prefix_same_as_start(true);
        let mut iter = self.raw_iterator_cf_opt(cf, opts);
        iter.seek(prefix);
        iter
    }
}
