pub mod compaction;
pub mod db;
pub mod manifest;
pub mod memtable;
pub mod sstable;
pub mod types;
pub mod util;
pub mod wal;

pub use db::Db;
pub use types::{ForgeError, Result};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::Db;

    fn temp_dir() -> PathBuf {
        let base = std::env::temp_dir();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        base.join(format!("forge_engine_test_{ts}"))
    }

    #[test]
    fn put_get_delete_roundtrip() {
        let dir = temp_dir();
        let mut db = Db::open(&dir).expect("open");

        db.put("a", b"1".to_vec()).expect("put");
        db.put("b", b"2".to_vec()).expect("put");
        assert_eq!(db.get("a").expect("get"), Some(b"1".to_vec()));

        db.delete("a").expect("delete");
        assert_eq!(db.get("a").expect("get"), None);

        db.sync().expect("sync");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recovers_from_wal() {
        let dir = temp_dir();

        {
            let mut db = Db::open(&dir).expect("open");
            db.put("name", b"forge".to_vec()).expect("put");
            db.put("version", b"0.1".to_vec()).expect("put");
        }

        let db = Db::open(&dir).expect("reopen");
        assert_eq!(db.get("name").expect("get"), Some(b"forge".to_vec()));
        assert_eq!(db.get("version").expect("get"), Some(b"0.1".to_vec()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recovers_tables_from_manifest() {
        let dir = temp_dir();

        {
            let mut db = Db::open(&dir).expect("open");
            db.put("persisted", b"yes".to_vec()).expect("put");
            db.sync().expect("sync");
        }

        let db = Db::open(&dir).expect("reopen");
        assert_eq!(db.get("persisted").expect("get"), Some(b"yes".to_vec()));

        let _ = fs::remove_dir_all(dir);
    }
}
