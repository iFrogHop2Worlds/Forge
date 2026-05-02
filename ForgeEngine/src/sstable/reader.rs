use std::fs::File;
use std::io::{BufReader, ErrorKind, Seek, SeekFrom};
use std::path::Path;

use crate::sstable::bloom::BloomFilter;
use crate::sstable::block::read_entry;
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result};
use crate::sstable::table::TableCache;

/// Iterates over entries in an SSTable file.
///
/// # Fields
///
/// * `reader` - A buffered reader over the SSTable data file.
/// * `current` - The entry currently exposed by the iterator, or `None` when the
///   iterator is invalid.
/// * `done` - Indicates whether the iterator has reached the end of the SSTable.
///
/// # Behavior
///
/// - The iterator reads entries sequentially from the SSTable.
/// - The first entry is loaded during `open`, so a newly opened iterator is ready
///   to read when `valid` returns `true`.
/// - End-of-file is treated as normal iterator exhaustion.
#[derive(Debug)]
pub struct SstableIterator {
    reader: BufReader<File>,
    current: Option<Entry>,
    done: bool,
}

impl SstableIterator {
    /// Opens an SSTable iterator for the file at the provided path.
    ///
    /// # Parameters
    /// - `path`: The path to the SSTable data file.
    ///
    /// # Returns
    /// - `Result<Self>`: Returns an initialized `SstableIterator` on success,
    ///   or an error if the file cannot be opened or the first entry cannot be read.
    ///
    /// # Behavior
    /// - Opens the SSTable file using a buffered reader.
    /// - Advances to the first entry before returning.
    /// - Marks the iterator invalid if the SSTable is empty.
    ///
    /// # Errors
    /// - Returns an error if the SSTable file cannot be opened.
    /// - Returns an error if the first entry is malformed or cannot be read.
    pub fn open(path: &Path) -> Result<Self> {
        let mut this = Self {
            reader: BufReader::new(File::open(path)?),
            current: None,
            done: false,
        };
        this.advance()?;
        Ok(this)
    }

    /// Indicates whether the iterator currently points to an entry.
    ///
    /// # Returns
    /// - `bool`: Returns `true` when `value` can be called safely, or `false`
    ///   when the iterator has reached the end of the SSTable.
    pub fn valid(&self) -> bool {
        self.current.is_some()
    }

    /// Returns the current SSTable entry.
    ///
    /// # Returns
    /// - `&Entry`: A shared reference to the current entry.
    ///
    /// # Panics
    /// - Panics if the iterator is invalid. Call `valid` before calling this method.
    pub fn value(&self) -> &Entry {
        self.current
            .as_ref()
            .expect("invalid sstable iterator access")
    }

    /// Advances the iterator to the next SSTable entry.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` when the iterator advances or reaches the
    ///   end of the SSTable, or an error if reading the next entry fails.
    ///
    /// # Errors
    /// - Returns an error if the next entry is malformed or cannot be read.
    pub fn next(&mut self) -> Result<()> {
        self.advance()
    }

    fn advance(&mut self) -> Result<()> {
        if self.done {
            self.current = None;
            return Ok(());
        }

        match read_entry(&mut self.reader) {
            Ok(entry) => {
                self.current = Some(entry);
            }
            Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                self.current = None;
                self.done = true;
            }
            Err(err) => return Err(err),
        }

        Ok(())
    }
}

/// Reads every entry from an SSTable file.
///
/// # Parameters
/// - `path`: The path to the SSTable data file.
///
/// # Returns
/// - `Result<Vec<Entry>>`: Returns all decoded entries on success, or an error
///   if the SSTable cannot be opened or decoded.
///
/// # Behavior
/// - Opens the SSTable file using a buffered reader.
/// - Reads entries sequentially until end-of-file is reached.
/// - Treats end-of-file as the normal completion condition.
///
/// # Errors
/// - Returns an error if the SSTable file cannot be opened.
/// - Returns an error if any entry is malformed or cannot be read.
pub fn read_all(path: &Path) -> Result<Vec<Entry>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut out = Vec::new();

    loop {
        match read_entry(&mut reader) {
            Ok(entry) => out.push(entry),
            Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                break;
            }
            Err(err) => return Err(err),
        }
    }

    Ok(out)
}

/// Looks up a key in an SSTable using its bloom filter and sparse index.
///
/// # Parameters
/// - `path`: The path to the SSTable data file.
/// - `index_path`: The path to the sparse index file associated with the SSTable.
/// - `bloom_path`: The path to the bloom filter file associated with the SSTable.
/// - `key`: The key to search for.
///
/// # Returns
/// - `Result<Option<Entry>>`: Returns `Ok(Some(Entry))` when the key is found,
///   `Ok(None)` when the key is absent, or an error if the SSTable or index cannot
///   be read.
///
/// # Behavior
/// - Loads the bloom filter from `bloom_path`.
/// - Rejects the lookup immediately when the bloom filter says the key is definitely absent.
/// - Loads the sparse index only when the bloom filter returns "maybe".
/// - Seeks to the greatest indexed offset less than or equal to `key`.
/// - Scans forward until the key is found, a greater key is encountered, or the
///   end of the SSTable is reached.
///
/// # Errors
/// - Returns an error if the index or SSTable file cannot be opened or read.
/// - Returns an error if an entry or index record is malformed.
pub fn get(path: &Path, index_path: &Path, bloom_path: &Path, key: &str) -> Result<Option<Entry>> {
    let bloom = BloomFilter::load(bloom_path)?;
    if !bloom.might_contain(key) {
        return Ok(None);
    }

    let index = SparseIndex::load(index_path)?;
    let mut reader = BufReader::new(File::open(path)?);
    let start = index.floor_offset_for(key);
    reader.seek(SeekFrom::Start(start))?;

    loop {
        match read_entry(&mut reader) {
            Ok(entry) => {
                if entry.key == key {
                    return Ok(Some(entry));
                }
                if entry.key.as_str() > key {
                    return Ok(None);
                }
            }
            Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(err) => return Err(err),
        }
    }
}

/// Looks up a key using a cached SSTable table state.
///
/// # Behavior
/// - Uses the cached bloom filter and sparse index already loaded at startup.
/// - Clones the cached SSTable data handle for the read without reopening the file by path.
pub fn get_cached(table: &TableCache, key: &str) -> Result<Option<Entry>> {
    if !table.bloom().might_contain(key) {
        return Ok(None);
    }

    if let Some(entry) = table.get_from_block_index(key)? {
        return Ok(Some(entry));
    }

    let start = table.index().floor_offset_for(key);
    table.scan_from_offset(start, key)
}

/// Looks up a key using cached SSTable metadata but bypasses the decoded block LRU.
pub fn get_uncached(table: &TableCache, key: &str) -> Result<Option<Entry>> {
    if !table.bloom().might_contain(key) {
        return Ok(None);
    }

    if let Some(entry) = table.get_from_block_index_uncached(key)? {
        return Ok(Some(entry));
    }

    let start = table.index().floor_offset_for(key);
    table.scan_from_offset(start, key)
}
