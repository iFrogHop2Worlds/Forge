use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use crate::types::Result;
use crate::util::{read_u32, read_u64, write_u32, write_u64};

/// Describes a fixed-size SSTable block.
#[derive(Debug, Clone)]
pub struct BlockIndexEntry {
    pub first_key: String,
    pub offset: u64,
    pub entry_count: u32,
    pub byte_len: u32,
}

/// Persistent block index for an SSTable.
#[derive(Debug, Clone)]
pub struct BlockIndex {
    pub block_size: u32,
    pub entries: Vec<BlockIndexEntry>,
}

impl BlockIndex {
    /// Creates a new block index.
    pub fn new(block_size: u32, entries: Vec<BlockIndexEntry>) -> Self {
        Self { block_size, entries }
    }

    /// Saves the block index to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        write_u32(&mut writer, self.block_size)?;
        write_u32(&mut writer, self.entries.len() as u32)?;
        for e in &self.entries {
            write_u32(&mut writer, e.first_key.len() as u32)?;
            writer.write_all(e.first_key.as_bytes())?;
            write_u64(&mut writer, e.offset)?;
            write_u32(&mut writer, e.entry_count)?;
            write_u32(&mut writer, e.byte_len)?;
        }
        writer.flush()?;
        Ok(())
    }

    /// Loads the block index from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let block_size = read_u32(&mut reader)?;
        let count = read_u32(&mut reader)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let len = read_u32(&mut reader)? as usize;
            let mut key = vec![0u8; len];
            reader.read_exact(&mut key)?;
            let first_key = String::from_utf8(key).map_err(|_| {
                crate::types::ForgeError::Corruption("non-utf8 key in block index".to_string())
            })?;
            let offset = read_u64(&mut reader)?;
            let entry_count = read_u32(&mut reader)?;
            let byte_len = read_u32(&mut reader)?;
            entries.push(BlockIndexEntry { first_key, offset, entry_count, byte_len });
        }
        Ok(Self { block_size, entries })
    }

    /// Returns the block offset that should be searched for a key.
    pub fn floor_block_offset_for(&self, key: &str) -> Option<u64> {
        self.floor_block_entry_for(key).map(|entry| entry.offset)
    }

    /// Returns the block entry that should be searched for a key.
    pub fn floor_block_entry_for(&self, key: &str) -> Option<&BlockIndexEntry> {
        match self
            .entries
            .binary_search_by(|entry| entry.first_key.as_str().cmp(key))
        {
            Ok(idx) => self.entries.get(idx),
            Err(0) => None,
            Err(idx) => self.entries.get(idx - 1),
        }
    }
}
