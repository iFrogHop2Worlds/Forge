use std::fs;
use std::path::{Path, PathBuf};

use crate::compaction::{CompactionIterator, DbIterator, MergeIterator, TableIterator};
use crate::manifest::{Manifest, TableMeta};
use crate::memtable::MemTable;
use crate::sstable::{reader, writer};
use crate::types::{Entry, Result, ValueRef};
use crate::wal::Wal;

const DEFAULT_MEMTABLE_LIMIT_BYTES: usize = 4 * 1024 * 1024;
const MAX_LEVEL: usize = 3;
const LEVEL_COMPACTION_THRESHOLD: [usize; MAX_LEVEL + 1] = [16, 16, 16, usize::MAX];

#[derive(Debug)]
pub struct Db {
    dir: PathBuf,
    wal: Wal,
    memtable: MemTable,
    levels: Vec<Vec<TableMeta>>,
    next_seq: u64,
    next_table_id: u64,
    memtable_limit_bytes: usize,
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let dir = path.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;

        let manifest = match Manifest::load(&dir, MAX_LEVEL + 1)? {
            Some(m) => m,
            None => {
                let m = Self::build_manifest_from_disk(&dir)?;
                m.save(&dir)?;
                m
            }
        };

        let levels = manifest.levels;
        let next_table_id = manifest.next_table_id;

        let mut memtable = MemTable::new();
        let wal_path = dir.join("current.wal");
        let replayed = Wal::replay(&wal_path)?;
        let mut next_seq = 1u64;
        for entry in replayed {
            next_seq = next_seq.max(entry.seq + 1);
            memtable.insert(entry.seq, entry.key, entry.value);
        }

        let wal = Wal::open(&dir)?;

        Ok(Self {
            dir,
            wal,
            memtable,
            levels,
            next_seq,
            next_table_id,
            memtable_limit_bytes: DEFAULT_MEMTABLE_LIMIT_BYTES,
        })
    }

    pub fn put(&mut self, key: impl Into<String>, value: impl Into<Vec<u8>>) -> Result<()> {
        let entry = Entry {
            key: key.into(),
            value: ValueRef::Value(value.into()),
            seq: self.next_seq,
        };
        self.next_seq += 1;

        self.wal.append(&entry)?;
        self.memtable.insert(entry.seq, entry.key, entry.value);

        if self.memtable.approx_bytes() >= self.memtable_limit_bytes {
            self.flush_memtable()?;
        }

        Ok(())
    }

    pub fn delete(&mut self, key: impl Into<String>) -> Result<()> {
        let entry = Entry {
            key: key.into(),
            value: ValueRef::Tombstone,
            seq: self.next_seq,
        };
        self.next_seq += 1;

        self.wal.append(&entry)?;
        self.memtable.insert(entry.seq, entry.key, entry.value);

        if self.memtable.approx_bytes() >= self.memtable_limit_bytes {
            self.flush_memtable()?;
        }

        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        if let Some((_, v)) = self.memtable.get(key) {
            return match v {
                ValueRef::Value(bytes) => Ok(Some(bytes)),
                ValueRef::Tombstone => Ok(None),
            };
        }

        for level in &self.levels {
            for t in level {
                if let Some(entry) = reader::get(&t.data_path, &t.index_path, key)? {
                    return match entry.value {
                        ValueRef::Value(v) => Ok(Some(v)),
                        ValueRef::Tombstone => Ok(None),
                    };
                }
            }
        }

        Ok(None)
    }

    pub fn flush_memtable(&mut self) -> Result<()> {
        if self.memtable.is_empty() {
            return Ok(());
        }

        self.wal.flush()?;

        let id = self.next_table_id;
        self.next_table_id += 1;
        let meta = TableMeta::new(&self.dir, id, 0);

        let entries: Vec<Entry> = self
            .memtable
            .iter_sorted()
            .map(|(k, seq, value)| Entry { key: k, value, seq })
            .collect();

        writer::write_sstable(&meta.data_path, &meta.index_path, &entries, 16)?;
        self.levels[0].insert(0, meta);
        self.persist_manifest()?;

        self.memtable.clear();
        self.wal.reset()?;

        self.maybe_compact_from_level(0)
    }

    fn maybe_compact_from_level(&mut self, start_level: usize) -> Result<()> {
        for level in start_level..MAX_LEVEL {
            if self.levels[level].len() >= LEVEL_COMPACTION_THRESHOLD[level] {
                self.compact_level_into_next(level)?;
            }
        }
        Ok(())
    }

    fn compact_level_into_next(&mut self, level: usize) -> Result<()> {
        let next_level = level + 1;

        let mut table_iters: Vec<Box<dyn DbIterator>> = Vec::new();

        for t in &self.levels[next_level] {
            table_iters.push(Box::new(TableIterator::open(&t.data_path)?));
        }

        for t in &self.levels[level] {
            table_iters.push(Box::new(TableIterator::open(&t.data_path)?));
        }

        let merge_iter = MergeIterator::new(table_iters);
        let mut compaction_iter = CompactionIterator::new(merge_iter);

        let id = self.next_table_id;
        self.next_table_id += 1;
        let output = TableMeta::new(&self.dir, id, next_level as u8);

        let mut compacted = Vec::new();
        while compaction_iter.valid() {
            compacted.push(compaction_iter.value().clone());
            compaction_iter.next();
        }

        writer::write_sstable(&output.data_path, &output.index_path, &compacted, 32)?;

        let old_level_tables: Vec<_> = self.levels[level].drain(..).collect();
        let old_next_level_tables: Vec<_> = self.levels[next_level].drain(..).collect();

        for old in old_level_tables.into_iter().chain(old_next_level_tables.into_iter()) {
            let _ = fs::remove_file(old.data_path);
            let _ = fs::remove_file(old.index_path);
        }

        self.levels[next_level].push(output);
        self.levels[next_level].sort_by(|a, b| b.id.cmp(&a.id));
        self.persist_manifest()?;

        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.flush_memtable()
    }

    fn persist_manifest(&self) -> Result<()> {
        let manifest = Manifest {
            next_table_id: self.next_table_id,
            levels: self.levels.clone(),
        };
        manifest.save(&self.dir)
    }

    fn build_manifest_from_disk(dir: &Path) -> Result<Manifest> {
        let mut manifest = Manifest::empty(MAX_LEVEL + 1);
        let mut max_id = 0;

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let p = entry.path();
            if p.extension().and_then(|x| x.to_str()) != Some("sst") {
                continue;
            }

            let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or_default();
            let mut parts = stem.split('_');
            let level_str = parts.next().unwrap_or_default();
            let id_str = parts.next().unwrap_or_default();
            if !level_str.starts_with('L') {
                continue;
            }

            let level: usize = level_str[1..].parse().unwrap_or(0);
            let id: u64 = id_str.parse().unwrap_or(0);

            if level > MAX_LEVEL || id == 0 {
                continue;
            }

            manifest.levels[level].push(TableMeta::new(dir, id, level as u8));
            max_id = max_id.max(id);
        }

        for level in &mut manifest.levels {
            level.sort_by(|a, b| b.id.cmp(&a.id));
        }

        manifest.next_table_id = (max_id + 1).max(1);
        Ok(manifest)
    }
}
