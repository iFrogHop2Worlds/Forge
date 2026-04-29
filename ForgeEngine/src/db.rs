use std::fs;
use std::path::{Path, PathBuf};

use crate::compaction::{CompactionIterator, DbIterator, MergeIterator, TableIterator};
use crate::manifest::{Manifest, TableMeta};
use crate::memtable::MemTable;
use crate::sstable::{reader, writer};
use crate::types::{Entry, Result, ValueRef};
use crate::wal::Wal;

const DEFAULT_MEMTABLE_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const MAX_LEVEL: usize = 9;
const LEVEL_COMPACTION_THRESHOLD: [usize; MAX_LEVEL + 1] =
    [16, 10, 10, 10, 10, 10, 10, 10, 10, usize::MAX];

/// The core structure of the Forge data engine
/// This struct is the backbone of a log structured merge tree (LSM-tree) based engine,
/// ensuring high performance and durability. Managing data storage, in-memory tables,
/// write-ahead logging (WAL), and multilevel organization of immutable tables.
///
/// # Fields
///
/// * `dir` - The directory path where the database files are stored.
/// * `wal` - The Write-Ahead Log (WAL) used for crash recovery and ensuring durability.
/// * `memtable` - The in-memory table used for fast data reads and writes before flushing
///   to disk.
/// * `levels` - A vector of levels, where each level contains a vector of `TableMeta`
///   objects representing immutable tables stored on disk.
/// * `next_seq` - The sequence number for tracking and ordering operations in the database.
/// * `next_table_id` - The identifier to be assigned to the next table created in the database.
/// * `memtable_limit_bytes` - The size limit, in bytes, for the in-memory table before it is
///   flushed to a new immutable table on disk.
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
    /// Opens an instance of the database from the specified path. If the database directory
    /// does not exist, it will be created. The method ensures the database's integrity by
    /// loading a manifest file (or rebuilding it from disk if not found) and replaying
    /// the Write-Ahead Log (WAL) to reconstruct the current state of the in-memory table (MemTable).
    ///
    /// # Arguments
    ///
    /// * `path` - A path (or a type that can be referenced as a path) pointing to the database directory.
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` containing the initialized database instance if successful, or an error if any of the
    /// file operations, manifest loading, or WAL replay fails.
    ///
    /// # Steps Performed
    ///
    /// 1. Ensures the directory specified by `path` exists, creating it if necessary.
    /// 2. Attempts to load the `Manifest` file from the directory:
    ///     - If the manifest is present, its contents are used to initialize the levels and metadata.
    ///     - If the manifest is absent, it is rebuilt by scanning the directory contents and subsequently saved.
    /// 3. Creates an empty `MemTable`.
    /// 4. Reads and replays the WAL file (`current.wal`) to populate the `MemTable` with previously persisted operations.
    /// 5. Opens a new WAL instance for future writes.
    ///
    /// # Errors
    ///
    /// This function may return an error in the following cases:
    ///
    /// - Failure to create the database directory.
    /// - Issues while loading or rebuilding the Manifest.
    /// - Failure to replay the WAL due to corruption or other I/O errors.
    /// - Inability to open a new WAL for writing.
    ///
    /// # Example
    ///
    /// ```
    /// use ForgeEngine::Db;
    ///
    /// let db = Db::open("path/to/db").expect("Failed to open database");
    /// ```
    ///
    /// # Notes
    ///
    /// - The database starts with an empty `MemTable`, but its state is populated by replaying the WAL entries.
    /// - The next sequence number for transactions (`next_seq`) and the next table ID (`next_table_id`) are updated
    ///   based on the persisted WAL and manifest data.
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

    /// Inserts a key-value pair into the database, assigning it a unique sequence number.
    ///
    /// # Arguments
    ///
    /// * `key` - A value that can be converted into a `String`, representing the key of the entry.
    /// * `value` - A value that can be converted into a `Vec<u8>`, representing the value of the entry.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the insertion was successful.
    /// * `Err` if an error occurred during the operation, such as failure to append to the write-ahead log (WAL)
    ///   or a failure during flushing the memtable to persistent storage.
    ///
    /// # Behavior
    ///
    /// 1. A new `Entry` is created using the provided `key` and `value`, along with the next available sequence number (`self.next_seq`).
    /// 2. The sequence number is incremented for the next entry.
    /// 3. The entry is appended to the write-ahead log (WAL) for durability.
    /// 4. The key-value pair is inserted into the in-memory structure (`memtable`).
    /// 5. If the approximate memory usage of the `memtable` exceeds the configured limit (`self.memtable_limit_bytes`),
    ///    the `flush_memtable` method is invoked to persist its contents and free memory.
    ///
    /// # Errors
    ///
    /// This method propagates errors from:
    ///
    /// * The `wal.append` operation, if there is an issue appending the entry to the write-ahead log.
    /// * The `flush_memtable` method, if there is an issue persisting the memtable to secondary storage.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut db = Database::new();
    /// db.put("key1", b"value1").unwrap();
    /// db.put("key2", b"value2").unwrap();
    /// ```
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

    /// Deletes a key-value pair from the storage system by marking the key as a tombstone
    /// the value will be ignored on reads and will be dropped on the next compaction.
    ///
    /// # Parameters
    /// - `key`: The key to delete. Accepts any type that can be converted into a `String`.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` on success, or an error if the operation fails.
    ///
    /// # Behavior
    /// 1. Constructs an `Entry` with the provided key, a tombstone value, and the next available sequence number.
    /// 2. Increments the sequence number counter (`next_seq`).
    /// 3. Appends the delete operation to the write-ahead log (`wal`).
    /// 4. Inserts the tombstone entry into the in-memory storage structure (`memtable`).
    /// 5. Checks if the approximate memory usage of `memtable` exceeds the predefined limit (`memtable_limit_bytes`).
    ///    - If the limit is exceeded, triggers the flushing of the `memtable` to persistent storage.
    ///
    /// # Errors
    /// - Returns an error if appending to the write-ahead log fails.
    /// - Returns an error if flushing the `memtable` fails when the memory limit is exceeded.
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

    /// Retrieves the value associated with the specified `key` from the database.
    ///
    /// This function searches for the key in the in-memory `memtable` first and, if not found,
    /// proceeds to look through the levels of on-disk tables. Depending on the value associated
    /// with the key, it returns one of the following:
    ///
    /// - `Ok(Some(Vec<u8>))`: The key exists and has an associated value.
    /// - `Ok(None)`: The key exists but has been marked with a tombstone, indicating deletion.
    /// - `Ok(None)`: The key does not exist in the database.
    /// - `Err(e)`: An error occurred during retrieval, e.g., an I/O issue while accessing
    ///   on-disk tables.
    ///
    /// # Arguments
    ///
    /// * `key` - A string slice representing the key to look up in the database.
    ///
    /// # Returns
    ///
    /// * `Result<Option<Vec<u8>>>`:
    ///     - `Ok(Some(Vec<u8>))` if the key is found with a value.
    ///     - `Ok(None)` if the key is found with a tombstone or does not exist.
    ///     - `Err(e)` if an error occurs during the process.
    ///
    /// # Behavior
    ///
    /// 1. The function first checks the in-memory `memtable` for the key.
    ///    - If found, it distinguishes whether the key has an associated value (`ValueRef::Value`)
    ///      or has been logically deleted (`ValueRef::Tombstone`).
    /// 2. If the key is not present in the `memtable`, it scans the on-disk levels.
    ///    - Each level is searched sequentially, and the on-disk tables are accessed using the
    ///      `reader::get` function.
    ///    - If the key is located in these tables, the function behaves similarly to the `memtable`
    ///      lookup, differentiating based on `ValueRef`.
    /// 3. If the key is not found in either the `memtable` or the on-disk tables, the function
    ///    returns `Ok(None)`.
    ///
    /// # Errors
    ///
    /// This function may return an error if an issue occurs while accessing the on-disk tables,
    /// such as a file I/O error during the execution of `reader::get`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// match db.get("my_key") {
    ///     Ok(Some(value)) => println!("Found value: {:?}", value),
    ///     Ok(None) => println!("Key not found or marked as deleted."),
    ///     Err(e) => eprintln!("Error retrieving key: {:?}", e),
    /// }
    /// ```
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

    /// Flushes the current in-memory memtable to persistent storage by streaming it to a new
    /// SSTable and updating the associated metadata and indices.
    ///
    /// # Workflow
    /// 1. Checks if the memtable is empty; if so, the function returns `Ok(())` without taking any action.
    /// 2. Flushes the Write-Ahead Log (WAL) to ensure all operations in the log are safely persisted.
    /// 3. Generates a new table ID and creates metadata for the new SSTable.
    /// 4. Streams borrowed entries from the memtable in sorted key order.
    /// 5. Calls the `writer::write_sstable_refs` function to write the borrowed entries to disk,
    ///    creating both the data file and the index file for the new SSTable.
    /// 6. Updates the metadata for level 0 of the storage hierarchy to include the newly created SSTable.
    /// 7. Persists the updated manifest file containing the storage system's structure.
    /// 8. Clears the in-memory memtable, resetting its state for future use.
    /// 9. Resets the WAL to prepare for future writes.
    /// 10. Initiates a background compaction process starting from level 0 if necessary.
    ///
    /// # Returns
    /// * `Result<()>`: Returns `Ok(())` if the flush operation is successful. If any stage fails,
    ///   the function will return an appropriate error.
    ///
    /// # Errors
    /// This function can return an `Err` in the following conditions:
    /// * If flushing the WAL fails.
    /// * If streaming the borrowed memtable entries to the SSTable or index file fails.
    /// * If persisting the manifest fails.
    /// * If an issue occurs during memtable clearance or WAL reset.
    ///
    /// # Panics
    /// This function does not explicitly panic, but it relies on external functions that may panic
    /// in exceptional circumstances.
    pub fn flush_memtable(&mut self) -> Result<()> {
        if self.memtable.is_empty() {
            return Ok(());
        }

        self.wal.flush()?;

        let id = self.next_table_id;
        self.next_table_id += 1;
        let meta = TableMeta::new(&self.dir, id, 0);

        writer::write_sstable_refs(
            &meta.data_path,
            &meta.index_path,
            self.memtable.iter_sorted_ref(),
            16,
        )?;
        self.levels[0].insert(0, meta);
        self.persist_manifest()?;

        self.memtable.clear();
        self.wal.reset()?;

        self.maybe_compact_from_level(0)
    }

    /// Checks each level starting from the specified `start_level` to determine if a compaction
    /// is needed, and if so, performs the compaction by merging the current level into the next one.
    ///
    /// # Parameters
    /// - `start_level`: The starting level index from which to check and potentially compact.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` if all levels are checked and compacted as necessary,
    ///   or an error if encountered during the compaction process.
    ///
    /// # Behavior
    /// For each level starting from `start_level` up to `MAX_LEVEL`:
    /// - If the number of entries in the level exceeds or equals the corresponding threshold
    ///   defined in `LEVEL_COMPACTION_THRESHOLD[level]`, the function invokes
    ///   `compact_level_into_next(level)` to merge the contents of the current level into the next.
    ///
    /// # Errors
    /// This function will return an error in the event that `compact_level_into_next(level)` fails.
    fn maybe_compact_from_level(&mut self, start_level: usize) -> Result<()> {
        for level in start_level..MAX_LEVEL {
            if self.levels[level].len() >= LEVEL_COMPACTION_THRESHOLD[level] {
                self.compact_level_into_next(level)?;
            }
        }
        Ok(())
    }

    /// Compacts the SSTables at the specified level into the next level, merging and reorganizing
    /// the data to optimize storage and query efficiency.
    ///
    /// # Arguments
    ///
    /// * `level` - The level of SSTables (Sorted String Tables) to compact.
    ///
    /// # Workflow
    ///
    /// 1. Determines the next level to which the SSTables will be compacted.
    /// 2. Opens iterators for all SSTables at both the current (`level`) and the next level (`level + 1`).
    /// 3. Uses a `MergeIterator` to combine and merge the SSTables.
    /// 4. Passes the merged iterator into a `CompactionIterator` for further processing.
    /// 5. Creates a new SSTable at the next level to store the compacted entries.
    /// 6. Merges the entries into a single sequence and serializes the data into the new SSTable using the `write_sstable` utility.
    /// 7. Cleans up the old SSTable files from both the current and next levels by deleting their corresponding data and index files.
    /// 8. Updates the metadata to reflect the new SSTable in the next level:
    ///     - Adds the new `TableMeta` to the next level.
    ///     - Sorts the SSTables to maintain order based on their IDs.
    /// 9. Updates the manifest file to persist changes in the metadata.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If compaction was successful.
    /// * `Err` - If an error occurs during the process, such as file I/O errors or iterator creation issues.
    ///
    /// # Errors
    ///
    /// This function propagates errors that may occur during:
    ///
    /// * Opening or iterating over SSTables.
    /// * Writing the compacted data to a new SSTable.
    /// * Removing old SSTable files.
    /// * Persisting metadata to the manifest file.
    ///
    /// # Example
    ///
    /// ```rust
    /// // Example usage within a database context:
    /// db.compact_level_into_next(0)?;
    /// ```
    ///
    /// This example compacts the SSTables in level 0 into level 1.
    ///
    /// # Note
    ///
    /// - Compaction operations can be resource intensive and may temporarily increase storage usage.
    /// - Ensure sufficient disk space is available before performing the operation.
    ///
    /// # See Also
    ///
    /// - [`TableIterator`](struct.TableIterator.html)
    /// - [`MergeIterator`](struct.MergeIterator.html)
    /// - [`CompactionIterator`](struct.CompactionIterator.html)
    /// - [`write_sstable`](fn.writer::write_sstable.html)
    /// - [`TableMeta`](struct.TableMeta.html)
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

        for old in old_level_tables
            .into_iter()
            .chain(old_next_level_tables.into_iter())
        {
            let _ = fs::remove_file(old.data_path);
            let _ = fs::remove_file(old.index_path);
        }

        self.levels[next_level].push(output);
        self.levels[next_level].sort_by(|a, b| b.id.cmp(&a.id));
        self.persist_manifest()?;

        Ok(())
    }

    /// Synchronizes the current state by flushing the in-memory table (memtable) to ensure that
    /// any pending changes are written and persisted.
    ///
    /// # Returns
    ///
    /// * `Result<()>` - Returns `Ok(())` if the flushing operation succeeds, otherwise returns an error.
    ///
    /// # Errors
    ///
    /// This function will return an error if the underlying `flush_memtable` operation fails.
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

    /// Builds a `Manifest` by scanning the contents of a specified directory on disk.
    ///
    /// This function reads all the `.sst` files in the provided directory and parses their file names
    /// to extract metadata such as the level and table ID. The extracted metadata is then used to populate
    /// a `Manifest` structure, which serves as a mapping of table metadata by level. The `next_table_id`
    /// for the `Manifest` is set to one greater than the largest table ID found during the scan.
    ///
    /// # Arguments
    ///
    /// * `dir` - A reference to a `Path` that specifies the directory to scan for `.sst` files.
    ///
    /// # Returns
    ///
    /// * `Result<Manifest>` - A result containing the populated `Manifest` if successful, or an error
    ///   if any issues occur while reading the directory or parsing file information.
    ///
    /// # Behavior
    ///
    /// - Only files with the `.sst` extension are considered.
    /// - Files are expected to follow the naming format `L<level>_<id>.sst`, where:
    ///   - `<level>` is a zero-based level specifier prefixed by `L`.
    ///   - `<id>` is a positive numeric identifier for the table.
    /// - Files that do not conform to this naming convention are ignored.
    /// - The manifest is organized into levels, with tables sorted in descending order of their IDs
    ///   within each level.
    ///
    /// # Errors
    ///
    /// The function will return an error if:
    /// - The provided directory is invalid or cannot be read.
    /// - There are issues parsing or extracting metadata from the file system.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::Path;
    ///
    /// let dir = Path::new("/path/to/manifest/directory");
    /// match build_manifest_from_disk(dir) {
    ///     Ok(manifest) => println!("Manifest built successfully: {:?}", manifest),
    ///     Err(e) => eprintln!("Failed to build manifest: {}", e),
    /// }
    /// ```
    ///
    /// # Internal Details
    ///
    /// - The function initializes an empty `Manifest` with levels ranging from 0 to `MAX_LEVEL`.
    /// - Each `.sst` file is analyzed to extract its level and table ID. If a file's level exceeds
    ///   `MAX_LEVEL` or its ID is invalid (e.g., 0), it is skipped.
    /// - After populating the manifest, tables within each level are sorted by descending ID.
    /// - The `next_table_id` is computed as `(max_id + 1).max(1)` to ensure it's at least 1.
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
