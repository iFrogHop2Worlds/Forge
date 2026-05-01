use std::fs::File;
use std::path::Path;
use std::{collections::{HashMap, VecDeque}, sync::Mutex};

use crate::sstable::block_index::BlockIndex;
use crate::sstable::bloom::BloomFilter;
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result};

/// In-memory cache for one immutable SSTable.
///
/// # Behavior
/// - Holds the bloom filter, sparse index, and optional block index in memory for the lifetime of the table cache.
/// - Keeps one open data-file handle and clones it for reads so lookups do not reopen the file by path.
/// - Caches decoded blocks in a bounded LRU so repeated reads can skip decompression and entry decoding.
#[derive(Debug)]
pub struct TableCache {
    data_file: File,
    block_index: Option<BlockIndex>,
    bloom: BloomFilter,
    index: SparseIndex,
    block_cache: Mutex<BlockCache>,
}

#[derive(Debug)]
struct BlockCache {
    map: HashMap<u64, Vec<Entry>>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl BlockCache {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    fn get(&mut self, offset: u64) -> Option<Vec<Entry>> {
        let value = self.map.get(&offset).cloned();
        if value.is_some() {
            self.touch(offset);
        }
        value
    }

    fn insert(&mut self, offset: u64, block: Vec<Entry>) {
        if self.map.contains_key(&offset) {
            self.map.insert(offset, block);
            self.touch(offset);
            return;
        }

        if self.map.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }

        self.order.push_back(offset);
        self.map.insert(offset, block);
    }

    fn touch(&mut self, offset: u64) {
        if let Some(pos) = self.order.iter().position(|&x| x == offset) {
            self.order.remove(pos);
        }
        self.order.push_back(offset);
    }
}

impl TableCache {
    /// Loads the table metadata and opens the SSTable file once.
    ///
    /// # Notes
    /// - The bloom filter and sparse index are read into memory at startup or after a table rewrite.
    /// - Tables written before the block index existed still load successfully with only the sparse index.
    pub fn load(data_path: &Path, index_path: &Path, bloom_path: &Path) -> Result<Self> {
        Ok(Self {
            data_file: File::open(data_path)?,
            block_index: None,
            bloom: BloomFilter::load(bloom_path)?,
            index: SparseIndex::load(index_path)?,
            block_cache: Mutex::new(BlockCache::new(64)),
        })
    }

    /// Loads a cache with an available block index.
    ///
    /// # Notes
    /// - The block index is used to select one fixed-size block before falling back to the sparse index path.
    pub fn load_with_block_index(
        data_path: &Path,
        block_index_path: &Path,
        index_path: &Path,
        bloom_path: &Path,
    ) -> Result<Self> {
        Ok(Self {
            data_file: File::open(data_path)?,
            block_index: Some(BlockIndex::load(block_index_path)?),
            bloom: BloomFilter::load(bloom_path)?,
            index: SparseIndex::load(index_path)?,
            block_cache: Mutex::new(BlockCache::new(64)),
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

    /// Returns the cached block index if one is available.
    pub fn block_index(&self) -> Option<&BlockIndex> {
        self.block_index.as_ref()
    }

    /// Returns a cached decoded block if available.
    pub fn cached_block(&self, offset: u64) -> Option<Vec<Entry>> {
        self.block_cache.lock().ok().and_then(|mut cache| cache.get(offset))
    }

    /// Stores a decoded block in the cache.
    pub fn insert_block_cache(&self, offset: u64, block: Vec<Entry>) {
        if let Ok(mut cache) = self.block_cache.lock() {
            cache.insert(offset, block);
        }
    }

    /// Reads a block by offset and decodes the contained entries.
    pub fn load_block_entries(&self, offset: u64, byte_len: u32, entry_count: u32) -> Result<Vec<Entry>> {
        use std::io::{Cursor, Read, Seek, SeekFrom};

        let mut reader = std::io::BufReader::new(self.try_clone_data_file()?);
        reader.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; byte_len as usize];
        reader.read_exact(&mut buf)?;

        let mut cursor = Cursor::new(buf);
        let mut entries = Vec::with_capacity(entry_count as usize);
        for _ in 0..entry_count {
            entries.push(crate::sstable::block::read_entry(&mut cursor)?);
        }
        Ok(entries)
    }

    /// Searches the cached block index first, then loads and caches the block if needed.
    ///
    /// # Notes
    /// - This is the point-lookup fast path for new block-indexed SSTables.
    pub fn get_from_block_index(
        &self,
        key: &str,
    ) -> Result<Option<Entry>> {
        let block_index = match self.block_index() {
            Some(index) => index,
            None => return Ok(None),
        };

        let block_meta = match block_index.floor_block_entry_for(key) {
            Some(v) => v,
            None => return Ok(None),
        };

        if let Some(block) = self.cached_block(block_meta.offset) {
            return Ok(scan_block(block, key));
        }

        let entries = self.load_block_entries(block_meta.offset, block_meta.byte_len, block_meta.entry_count)?;
        self.insert_block_cache(block_meta.offset, entries.clone());
        Ok(scan_block(entries, key))
    }
}

fn scan_block(entries: Vec<Entry>, key: &str) -> Option<Entry> {
    for entry in entries {
        if entry.key == key {
            return Some(entry);
        }
        if entry.key.as_str() > key {
            return None;
        }
    }
    None
}
