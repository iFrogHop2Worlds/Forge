use std::borrow::Borrow;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::sstable::block::{write_entry, write_entry_ref};
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result, ValueRef};

const SSTABLE_WRITE_BUFFER_BYTES: usize = 8 * 1024 * 1024;

/// Writes a sorted string table (SSTable) and its associated sparse index to disk.
///
/// # Parameters
/// - `path`: The file path where the SSTable will be written.
/// - `index_path`: The file path where the sparse index will be stored.
/// - `entries`: A slice of `Entry` objects representing key-value pairs to be written to the SSTable.
/// - `index_stride`: The interval for including entries in the sparse index. For example, if `index_stride`
///   is 4, every fourth entry will be included in the sparse index.
///
/// # Returns
/// - `Result<()>`: Returns `Ok(())` on success, or an error if writing the SSTable or index fails.
///
/// # Behavior
/// - The function streams the key-value pairs from `entries` to the SSTable file specified in `path`.
/// - As it writes the SSTable, it records sparse index points at intervals defined by `index_stride`. Each
///   sparse index point consists of a key and its byte offset in the SSTable file.
/// - The sparse index is then written to the file specified in `index_path`.
///
/// # Errors
/// - Returns an error if there is an issue creating or writing to the SSTable or index files.
/// - Returns an error if `index_stride` is `0`, leading to invalid behavior.
///
/// # Example
/// ```
/// use std::path::Path;
///
/// let entries = vec![
///     Entry { key: "apple".to_string(), value: "fruit".to_string() },
///     Entry { key: "banana".to_string(), value: "fruit".to_string() },
///     Entry { key: "carrot".to_string(), value: "vegetable".to_string() },
/// ];
/// let path = Path::new("sstable.dat");
/// let index_path = Path::new("sparse_index.dat");
///
/// write_sstable(&path, &index_path, &entries, 2).expect("Failed to write SSTable");
/// ```
///
/// # Notes
/// - This function assumes that the entries are already sorted by their keys.
/// - `index_stride` must be greater than 0; otherwise, the sparse index would not be properly generated.
/// - This function delegates to the shared streaming writer used by iterator-based SSTable writes.
///
/// # Dependencies
/// - The function relies on the `write_entry`, `SparseIndex`, and `Entry` types or utilities, which must be
///   defined elsewhere in the codebase.
/// - Requires `BufWriter` to buffer the SSTable writes for efficiency.
pub fn write_sstable(
    path: &Path,
    index_path: &Path,
    entries: &[Entry],
    index_stride: usize,
) -> Result<()> {
    write_sstable_entries(
        path,
        index_path,
        entries.iter(),
        index_stride,
        Some(entries.len()),
    )
}

/// Writes a sorted string table (SSTable) from an entry iterator and its associated sparse index to disk.
///
/// # Parameters
/// - `path`: The file path where the SSTable will be written.
/// - `index_path`: The file path where the sparse index will be stored.
/// - `entries`: An iterator of `Entry` objects representing sorted key-value pairs to write.
/// - `index_stride`: The interval for including entries in the sparse index.
///
/// # Returns
/// - `Result<()>`: Returns `Ok(())` on success, or an error if writing the SSTable or index fails.
///
/// # Behavior
/// - Streams entries directly from the iterator into the SSTable file.
/// - Tracks byte offsets as entries are written instead of seeking for the current file position.
/// - Builds and persists the sparse index after the data file is flushed.
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
    entries: I,
    index_stride: usize,
) -> Result<()>
where
    I: IntoIterator<Item = Entry>,
{
    write_sstable_entries(path, index_path, entries, index_stride, None)
}

/// Writes a sorted string table (SSTable) from borrowed entry fields and its associated sparse index to disk.
///
/// # Parameters
/// - `path`: The file path where the SSTable will be written.
/// - `index_path`: The file path where the sparse index will be stored.
/// - `entries`: An iterator of borrowed entry fields in sorted key order.
/// - `index_stride`: The interval for including entries in the sparse index.
///
/// # Returns
/// - `Result<()>`: Returns `Ok(())` on success, or an error if writing the SSTable or index fails.
///
/// # Behavior
/// - Streams borrowed key/value fields directly into the SSTable file.
/// - Avoids constructing owned `Entry` values during memtable flush.
/// - Tracks byte offsets as entries are written and persists a sparse index.
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
    entries: I,
    index_stride: usize,
) -> Result<()>
where
    I: IntoIterator<Item = (&'a String, u64, &'a ValueRef)>,
{
    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(SSTABLE_WRITE_BUFFER_BYTES, file);

    let mut offset = 0u64;
    let index_stride = index_stride.max(1);
    let mut sparse = Vec::new();
    for (i, (key, seq, value)) in entries.into_iter().enumerate() {
        if i % index_stride == 0 {
            sparse.push((key.clone(), offset));
        }
        offset += write_entry_ref(&mut writer, seq, key, value)? as u64;
    }

    writer.flush()?;
    SparseIndex::new(sparse).save(index_path)?;
    Ok(())
}

fn write_sstable_entries<I>(
    path: &Path,
    index_path: &Path,
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

    let mut offset = 0u64;
    let index_stride = index_stride.max(1);
    let mut sparse = Vec::with_capacity(size_hint.map_or(0, |len| len.div_ceil(index_stride)));
    for (i, entry) in entries.into_iter().enumerate() {
        let entry = entry.borrow();
        if i % index_stride == 0 {
            sparse.push((entry.key.clone(), offset));
        }
        offset += write_entry(&mut writer, entry)? as u64;
    }

    writer.flush()?;
    SparseIndex::new(sparse).save(index_path)?;
    Ok(())
}
