use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use crate::types::Result;
use crate::util::{read_u32, read_u64, write_u32, write_u64};

/// Represents a sparse index for an SSTable file.
///
/// # Fields
///
/// * `entries` - A vector of key and byte-offset pairs. Each key identifies a
///   searchable anchor point in the SSTable, and each offset points to the byte
///   location where scanning can begin.
///
/// # Behavior
///
/// - The sparse index stores only selected SSTable keys instead of every key.
/// - Lookups use the greatest indexed key less than or equal to the target key
///   to find a starting offset for a sequential scan.
///
/// # Notes
///
/// - Entries are expected to be sorted by key in the same order as the SSTable.
/// - The index is persisted separately from the SSTable data file.
#[derive(Debug, Clone)]
pub struct SparseIndex {
    pub entries: Vec<(String, u64)>,
}

impl SparseIndex {
    /// Creates a new sparse index from the provided key-offset entries.
    ///
    /// # Parameters
    /// - `entries`: A vector of `(String, u64)` pairs where each string is an
    ///   indexed key and each offset is a byte position in the SSTable file.
    ///
    /// # Returns
    /// - `Self`: Returns a `SparseIndex` containing the provided entries.
    ///
    /// # Notes
    /// - The entries should be sorted by key for lookup behavior to be correct.
    pub fn new(entries: Vec<(String, u64)>) -> Self {
        Self { entries }
    }

    /// Saves the sparse index to disk.
    ///
    /// # Parameters
    /// - `path`: The file path where the sparse index will be written.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` on success, or an error if the index
    ///   cannot be written.
    ///
    /// # Behavior
    /// - Writes the number of index entries as a `u32`.
    /// - Writes each indexed key length, SSTable offset, and key byte sequence.
    /// - Flushes the buffered writer before returning.
    ///
    /// # Errors
    /// - Returns an error if the index file cannot be created.
    /// - Returns an error if any index data cannot be written or flushed.
    pub fn save(&self, path: &Path) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        write_u32(&mut writer, self.entries.len() as u32)?;
        for (key, offset) in &self.entries {
            write_u32(&mut writer, key.len() as u32)?;
            write_u64(&mut writer, *offset)?;
            writer.write_all(key.as_bytes())?;
        }
        writer.flush()?;
        Ok(())
    }

    /// Loads a sparse index from disk.
    ///
    /// # Parameters
    /// - `path`: The file path containing the persisted sparse index.
    ///
    /// # Returns
    /// - `Result<Self>`: Returns the loaded `SparseIndex` on success, or an error
    ///   if the index cannot be read or decoded.
    ///
    /// # Behavior
    /// - Reads the index entry count from the file.
    /// - Reads each key length, SSTable offset, and key byte sequence.
    /// - Converts each key byte sequence into a UTF-8 `String`.
    ///
    /// # Errors
    /// - Returns an error if the index file cannot be opened or read.
    /// - Returns a corruption error if an indexed key is not valid UTF-8.
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let count = read_u32(&mut reader)? as usize;
        let mut entries = Vec::with_capacity(count);

        for _ in 0..count {
            let key_len = read_u32(&mut reader)? as usize;
            let offset = read_u64(&mut reader)?;
            let mut key = vec![0u8; key_len];
            reader.read_exact(&mut key)?;
            let key = String::from_utf8(key).map_err(|_| {
                crate::types::ForgeError::Corruption("non-utf8 key in index".to_string())
            })?;
            entries.push((key, offset));
        }

        Ok(Self { entries })
    }

    /// Finds the SSTable byte offset that should be scanned for a key lookup.
    ///
    /// # Parameters
    /// - `key`: The target key being searched for in the SSTable.
    ///
    /// # Returns
    /// - `u64`: Returns the greatest indexed offset whose key is less than or
    ///   equal to `key`, or `0` when no index entry qualifies.
    ///
    /// # Behavior
    /// - Scans the sparse index entries in order.
    /// - Tracks the most recent offset whose key is less than or equal to `key`.
    /// - Stops once an indexed key is greater than the target key.
    ///
    /// # Notes
    /// - This method assumes the sparse index entries are sorted by key.
    pub fn floor_offset_for(&self, key: &str) -> u64 {
        match self
            .entries
            .binary_search_by(|(k, _)| k.as_str().cmp(key))
        {
            Ok(idx) => self.entries[idx].1,
            Err(0) => 0,
            Err(idx) => self.entries[idx - 1].1,
        }
    }
}
