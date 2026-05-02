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
    use std::time::Instant;
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

    #[test]
    fn flush_writes_bloom_filter_companion_file() {
        let dir = temp_dir();

        {
            let mut db = Db::open(&dir).expect("open");
            db.put("bloomed", b"value".to_vec()).expect("put");
            db.sync().expect("sync");
        }

        let bloom = dir.join("L0_1.bf");
        assert!(bloom.exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    #[ignore = "performance benchmark"]
    fn benchmark_million_random_writes_and_200k_reads() {
        const WRITE_COUNT: usize = 1_000_000;
        const READ_COUNT: usize = 200_000;

        fn next_u64(state: &mut u64) -> u64 {
            // Simple deterministic generator so the benchmark is reproducible.
            *state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *state
        }

        let dir = temp_dir();
        let mut db = Db::open(&dir).expect("open");
        let mut inserted_keys = Vec::with_capacity(WRITE_COUNT);
        let mut rng_state = 0x0123_4567_89ab_cdef_u64;

        let write_start = Instant::now();
        for _ in 0..WRITE_COUNT {
            let raw_key = next_u64(&mut rng_state);
            inserted_keys.push(raw_key);
            let key = raw_key.to_string();
            let value = raw_key.to_le_bytes().to_vec();
            db.put(key, value).expect("put");
        }
        db.sync().expect("sync");
        let write_elapsed = write_start.elapsed();

        let read_start = Instant::now();
        for _ in 0..READ_COUNT {
            let idx = (next_u64(&mut rng_state) as usize) % inserted_keys.len();
            let key = inserted_keys[idx].to_string();
            let value = db.get(&key).expect("get");
            assert!(value.is_some(), "missing inserted key {key}");
        }
        let read_elapsed = read_start.elapsed();

        println!(
            "benchmark: writes={} in {:?}, reads={} in {:?}",
            WRITE_COUNT, write_elapsed, READ_COUNT, read_elapsed
        );

        let _ = fs::remove_dir_all(dir);
    }
}
