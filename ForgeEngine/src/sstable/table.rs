use std::fs::File;
use std::path::Path;

use crate::sstable::bloom::BloomFilter;
use crate::sstable::index::SparseIndex;
use crate::types::Result;

/// In-memory cache for one immutable SSTable.
///
/// # Behavior
/// - Holds the bloom filter and sparse index in memory for the lifetime of the table cache.
/// - Keeps one open data-file handle and clones it for reads so lookups do not reopen the file by path.
#[derive(Debug)]
pub struct TableCache {
    data_file: File,
    bloom: BloomFilter,
    index: SparseIndex,
}

impl TableCache {
    /// Loads the table metadata and opens the SSTable file once.
    ///
    /// # Notes
    /// - The bloom filter and sparse index are read into memory at startup or after a table rewrite.
    pub fn load(data_path: &Path, index_path: &Path, bloom_path: &Path) -> Result<Self> {
        Ok(Self {
            data_file: File::open(data_path)?,
            bloom: BloomFilter::load(bloom_path)?,
            index: SparseIndex::load(index_path)?,
        })
    }

    /// Creates a new read handle to the table data file.
    ///
    /// # Notes
    /// - This clones the cached file handle rather than reopening the path.
    pub fn try_clone_data_file(&self) -> Result<File> {
        Ok(self.data_file.try_clone()?)
    }

    /// Returns the cached bloom filter.
    pub fn bloom(&self) -> &BloomFilter {
        &self.bloom
    }

    /// Returns the cached sparse index.
    pub fn index(&self) -> &SparseIndex {
        &self.index
    }
}
