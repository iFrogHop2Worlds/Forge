use std::borrow::Borrow;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::sstable::bloom::{BloomConfig, BloomFilter};
use crate::sstable::block_index::{BlockIndex, BlockIndexEntry};
use crate::sstable::block::{write_entry, write_entry_ref};
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result, ValueRef};

const SSTABLE_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const BLOCK_TARGET_BYTES: usize = 4096;

/// Writes a sorted string table (SSTable), block index, bloom filter, and sparse index to disk.
///
/// # Parameters
/// - `path`: The file path where the SSTable will be written.
/// - `index_path`: The file path where the sparse index will be stored.
/// - `block_index_path`: The file path where the block index will be stored.
/// - `bloom_path`: The file path where the bloom filter will be stored.
/// - `entries`: A slice of `Entry` objects representing key-value pairs to be written to the SSTable.
/// - `index_stride`: The interval for including entries in the sparse index. For example, if `index_stride`
///   is 4, every fourth entry will be included in the sparse index.
///
/// # Returns
/// - `Result<()>`: Returns `Ok(())` on success, or an error if writing the SSTable or index fails.
///
/// # Behavior
/// - The function streams the key-value pairs from `entries` to the SSTable file specified in `path`.
/// - Entries are grouped into approximately `BLOCK_TARGET_BYTES` blocks while writing, and each block
///   records its first key, byte offset, encoded byte length, and entry count.
/// - As it writes the SSTable, it also records sparse index points at intervals defined by `index_stride`.
/// - The block index, bloom filter, and sparse index are then written to their companion files.
///
/// # Errors
/// - Returns an error if there is an issue creating or writing to the SSTable or index files.
/// - Returns an error if `index_stride` is `0`, leading to invalid behavior.
///
/// # Example
/// ```ignore
/// use ForgeEngine::sstable::writer::write_sstable;
/// use ForgeEngine::sstable::bloom::BloomConfig;
/// use ForgeEngine::types::{Entry, ValueRef};
/// use std::path::Path;
///
/// let entries = vec![
///     Entry { key: "apple".to_string(), value: ValueRef::Value(b"fruit".to_vec()), seq: 1 },
///     Entry { key: "banana".to_string(), value: ValueRef::Value(b"fruit".to_vec()), seq: 2 },
///     Entry { key: "carrot".to_string(), value: ValueRef::Value(b"vegetable".to_vec()), seq: 3 },
/// ];
/// let path = Path::new("sstable.dat");
/// let index_path = Path::new("sparse_index.dat");
/// let bloom_path = Path::new("sstable.bf");
/// let bloom_config = BloomConfig::default();
///
/// write_sstable(&path, &index_path, &bloom_path, bloom_config, &entries, 2).expect("Failed to write SSTable");
/// ```
///
/// # Notes
/// - This function assumes that the entries are already sorted by their keys.
/// - `index_stride` must be greater than 0; otherwise, the sparse index would not be properly generated.
/// - The block size target is fixed at roughly 4 KiB for now; this is the boundary to refine before adding LRU caching.
/// - This function delegates to the shared streaming writer used by iterator-based SSTable writes.
///
/// # Dependencies
/// - The function relies on the `write_entry`, `SparseIndex`, and `Entry` types or utilities, which must be
///   defined elsewhere in the codebase.
/// - Requires `BufWriter` to buffer the SSTable writes for efficiency.
pub fn write_sstable(
    path: &Path,
    index_path: &Path,
    block_index_path: &Path,
    bloom_path: &Path,
    bloom_config: BloomConfig,
    entries: &[Entry],
    index_stride: usize,
) -> Result<()> {
    write_sstable_entries(
        path,
        index_path,
        block_index_path,
        bloom_path,
        bloom_config,
        entries.iter(),
        index_stride,
        Some(entries.len()),
    )
}

/// Writes a sorted string table (SSTable) from an entry iterator and its associated block index, bloom filter, and sparse index to disk.
///
/// # Parameters
/// - `path`: The file path where the SSTable will be written.
/// - `index_path`: The file path where the sparse index will be stored.
/// - `block_index_path`: The file path where the block index will be stored.
/// - `bloom_path`: The file path where the bloom filter will be stored.
/// - `entries`: An iterator of `Entry` objects representing sorted key-value pairs to write.
/// - `index_stride`: The interval for including entries in the sparse index.
///
/// # Returns
/// - `Result<()>`: Returns `Ok(())` on success, or an error if writing the SSTable or index fails.
///
/// # Behavior
/// - Streams entries directly from the iterator into the SSTable file.
/// - Tracks byte offsets as entries are written instead of seeking for the current file position.
/// - Builds and persists the block index, bloom filter, and sparse index after the data file is flushed.
///
/// # Errors
/// - Returns an error if the SSTable or index file cannot be created or written.
///
/// # Notes
/// - This function assumes the iterator yields entries sorted by key.
/// - This avoids requiring callers to collect all entries into a temporary `Vec<Entry>`.
pub fn write_sstable_iter<I>(
    path: &Path,
    index_path: &Path,
    block_index_path: &Path,
    bloom_path: &Path,
    bloom_config: BloomConfig,
    entries: I,
    index_stride: usize,
) -> Result<()>
where
    I: IntoIterator<Item = Entry>,
{
    write_sstable_entries(
        path,
        index_path,
        block_index_path,
        bloom_path,
        bloom_config,
        entries,
        index_stride,
        None,
    )
}

/// Writes a sorted string table (SSTable) from borrowed entry fields and its associated block index, bloom filter, and sparse index to disk.
///
/// # Parameters
/// - `path`: The file path where the SSTable will be written.
/// - `index_path`: The file path where the sparse index will be stored.
/// - `block_index_path`: The file path where the block index will be stored.
/// - `bloom_path`: The file path where the bloom filter will be stored.
/// - `entries`: An iterator of borrowed entry fields in sorted key order.
/// - `index_stride`: The interval for including entries in the sparse index.
///
/// # Returns
/// - `Result<()>`: Returns `Ok(())` on success, or an error if writing the SSTable or index fails.
///
/// # Behavior
/// - Streams borrowed key/value fields directly into the SSTable file.
/// - Avoids constructing owned `Entry` values during memtable flush.
/// - Tracks byte offsets as entries are written and persists a block index plus the other lookup metadata.
///
/// # Errors
/// - Returns an error if the SSTable or index file cannot be created or written.
///
/// # Notes
/// - This function assumes the iterator yields entries sorted by key.
/// - The sparse index still owns sampled keys, so only indexed keys are cloned.
pub fn write_sstable_refs<'a, I>(
    path: &Path,
    index_path: &Path,
    block_index_path: &Path,
    bloom_path: &Path,
    bloom_config: BloomConfig,
    entries: I,
    index_stride: usize,
) -> Result<()>
where
    I: IntoIterator<Item = (&'a String, u64, &'a ValueRef)>,
{
    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(SSTABLE_WRITE_BUFFER_BYTES, file);
    let entries = entries.into_iter();
    let (_, upper_bound) = entries.size_hint();
    let expected_keys = upper_bound.unwrap_or_else(|| entries.size_hint().0).max(1);

    let mut offset = 0u64;
    let index_stride = index_stride.max(1);
    let mut bloom = BloomFilter::builder_with_expected_keys(bloom_config, expected_keys);
    let mut sparse = Vec::new();
    let mut block_index = Vec::new();
    let mut block_keys: Vec<String> = Vec::new();
    let mut block_offset = 0u64;
    let mut block_entry_count = 0u32;
    let mut block_bytes = 0u32;
    for (i, (key, seq, value)) in entries.enumerate() {
        bloom.insert(key);
        if block_entry_count == 0 {
            block_offset = offset;
        }
        let entry_offset = offset;
        let entry_bytes = write_entry_ref(&mut writer, seq, key, value)? as u64;
        offset += entry_bytes;
        block_bytes = block_bytes.saturating_add(entry_bytes as u32);
        block_entry_count += 1;
        block_keys.push(key.clone());
        if i % index_stride == 0 {
            sparse.push((key.clone(), entry_offset));
        }

        if block_bytes as usize >= BLOCK_TARGET_BYTES {
            block_index.push(BlockIndexEntry {
                first_key: block_keys.first().cloned().unwrap_or_default(),
                offset: block_offset,
                entry_count: block_entry_count,
                byte_len: block_bytes,
            });
            block_keys.clear();
            block_entry_count = 0;
            block_bytes = 0;
        }
    }

    if block_entry_count > 0 {
        block_index.push(BlockIndexEntry {
            first_key: block_keys.first().cloned().unwrap_or_default(),
            offset: block_offset,
            entry_count: block_entry_count,
            byte_len: block_bytes,
        });
    }

    writer.flush()?;
    BlockIndex::new(BLOCK_TARGET_BYTES as u32, block_index).save(block_index_path)?;
    bloom.finish().save(bloom_path)?;
    SparseIndex::new(sparse).save(index_path)?;
    Ok(())
}

fn write_sstable_entries<I>(
    path: &Path,
    index_path: &Path,
    block_index_path: &Path,
    bloom_path: &Path,
    bloom_config: BloomConfig,
    entries: I,
    index_stride: usize,
    size_hint: Option<usize>,
) -> Result<()>
where
    I: IntoIterator,
    I::Item: Borrow<Entry>,
{
    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(SSTABLE_WRITE_BUFFER_BYTES, file);
    let entries = entries.into_iter();
    let expected_keys = size_hint
        .or_else(|| entries.size_hint().1)
        .unwrap_or_else(|| entries.size_hint().0)
        .max(1);

    let mut offset = 0u64;
    let index_stride = index_stride.max(1);
    let mut bloom = BloomFilter::builder_with_expected_keys(bloom_config, expected_keys);
    let mut sparse = Vec::with_capacity(size_hint.map_or(0, |len| len.div_ceil(index_stride)));
    let mut block_index = Vec::new();
    let mut block_keys: Vec<String> = Vec::new();
    let mut block_offset = 0u64;
    let mut block_entry_count = 0u32;
    let mut block_bytes = 0u32;
    for (i, entry) in entries.enumerate() {
        let entry = entry.borrow();
        bloom.insert(&entry.key);
        if block_entry_count == 0 {
            block_offset = offset;
        }
        let entry_offset = offset;
        let entry_bytes = write_entry(&mut writer, entry)? as u64;
        offset += entry_bytes;
        block_bytes = block_bytes.saturating_add(entry_bytes as u32);
        block_entry_count += 1;
        block_keys.push(entry.key.clone());
        if i % index_stride == 0 {
            sparse.push((entry.key.clone(), entry_offset));
        }

        if block_bytes as usize >= BLOCK_TARGET_BYTES {
            block_index.push(BlockIndexEntry {
                first_key: block_keys.first().cloned().unwrap_or_default(),
                offset: block_offset,
                entry_count: block_entry_count,
                byte_len: block_bytes,
            });
            block_keys.clear();
            block_entry_count = 0;
            block_bytes = 0;
        }
    }

    if block_entry_count > 0 {
        block_index.push(BlockIndexEntry {
            first_key: block_keys.first().cloned().unwrap_or_default(),
            offset: block_offset,
            entry_count: block_entry_count,
            byte_len: block_bytes,
        });
    }

    writer.flush()?;
    BlockIndex::new(BLOCK_TARGET_BYTES as u32, block_index).save(block_index_path)?;
    bloom.finish().save(bloom_path)?;
    SparseIndex::new(sparse).save(index_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sstable::index::SparseIndex;
    use crate::types::ValueRef;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let base = std::env::temp_dir();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        base.join(format!("forge_engine_{name}_{ts}"))
    }

    #[test]
    fn sparse_index_points_to_entry_start() {
        let dir = temp_path("sstable_writer");
        fs::create_dir_all(&dir).expect("dir");

        let data_path = dir.join("table.sst");
        let index_path = dir.join("table.index");
        let block_index_path = dir.join("table.block");
        let bloom_path = dir.join("table.bf");

        let entries = vec![
            Entry {
                key: "a".to_string(),
                value: ValueRef::Value(b"one".to_vec()),
                seq: 1,
            },
            Entry {
                key: "b".to_string(),
                value: ValueRef::Value(b"two".to_vec()),
                seq: 2,
            },
        ];

        write_sstable(
            &data_path,
            &index_path,
            &block_index_path,
            &bloom_path,
            BloomConfig::default(),
            &entries,
            1,
        )
        .expect("write");

        let sparse = SparseIndex::load(&index_path).expect("load index");
        assert_eq!(sparse.entries[0].0, "a");
        assert_eq!(sparse.entries[0].1, 0);
        assert!(sparse.entries[1].1 > sparse.entries[0].1);

        let _ = fs::remove_dir_all(dir);
    }
}
