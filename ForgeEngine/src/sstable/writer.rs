use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::sstable::block::write_entry;
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result};

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
/// - The function writes the key-value pairs from `entries` to the SSTable file specified in `path`.
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
    let file = File::create(path)?;
    let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, file);

    let mut offset = 0u64;
    let mut sparse = Vec::with_capacity(entries.len().div_ceil(index_stride.max(1)));
    for (i, entry) in entries.iter().enumerate() {
        if i % index_stride.max(1) == 0 {
            sparse.push((entry.key.clone(), offset));
        }
        offset += write_entry(&mut writer, entry)? as u64;
    }

    writer.flush()?;
    SparseIndex::new(sparse).save(index_path)?;
    Ok(())
}
