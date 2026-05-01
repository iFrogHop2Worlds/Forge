use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use crate::types::Result;
use crate::util::{read_u32, write_u32};

const BLOOM_MAGIC: u32 = 0x4642_4c4d; // FBLM
const BLOOM_VERSION: u32 = 1;
const DEFAULT_BITS_PER_KEY: usize = 8;
const DEFAULT_HASHES: u32 = 6;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// Compact membership filter for SSTable keys.
///
/// # Behavior
///
/// - The filter is built from the keys written into a table.
/// - A negative result is definitive: the key is not present.
/// - A positive result is probabilistic and may produce false positives.
///
/// # Notes
///
/// - Bloom filters are useful for point lookups and other existence checks.
/// - They do not encode ordering information, so they are not a substitute for
///   range metadata such as min/max key fences.
#[derive(Debug, Clone)]
pub struct BloomFilter {
    bits: Vec<u8>,
    bit_count: usize,
    hash_count: u32,
    entry_count: usize,
}

/// Tunable bloom filter parameters.
#[derive(Debug, Clone, Copy)]
pub struct BloomConfig {
    pub bits_per_key: usize,
    pub hash_count: u32,
}

impl BloomConfig {
    /// Returns a conservative default configuration.
    pub fn default() -> Self {
        Self {
            bits_per_key: DEFAULT_BITS_PER_KEY,
            hash_count: DEFAULT_HASHES,
        }
    }
}

/// Streaming builder for bloom filters.
#[derive(Debug)]
pub struct BloomBuilder {
    bits: Vec<u8>,
    bit_count: usize,
    hash_count: u32,
    entry_count: usize,
    bits_per_key: usize,
}

impl BloomFilter {
    /// Creates a streaming bloom builder.
    pub fn builder() -> BloomBuilder {
        BloomBuilder::new(BloomConfig::default())
    }

    /// Creates a streaming bloom builder with custom parameters.
    pub fn builder_with(config: BloomConfig) -> BloomBuilder {
        BloomBuilder::new(config)
    }

    /// Builds a bloom filter from the provided keys.
    pub fn new<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut builder = BloomBuilder::new(BloomConfig::default());
        for key in keys {
            builder.insert(key.as_ref());
        }
        builder.finish()
    }

    /// Checks whether a key may be present in the filter.
    pub fn might_contain(&self, key: &str) -> bool {
        let (h1, h2) = hash_pair(key);
        (0..self.hash_count).all(|i| self.test_bit(index_for_hashes(self.bit_count, h1, h2, i)))
    }

    /// Writes the bloom filter to disk.
    pub fn save(&self, path: &Path) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        write_u32(&mut writer, BLOOM_MAGIC)?;
        write_u32(&mut writer, BLOOM_VERSION)?;
        write_u32(&mut writer, self.hash_count)?;
        write_u32(&mut writer, self.bit_count as u32)?;
        write_u32(&mut writer, self.entry_count as u32)?;
        write_u32(&mut writer, self.bits.len() as u32)?;
        writer.write_all(&self.bits)?;
        writer.flush()?;
        Ok(())
    }

    /// Loads a bloom filter from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let magic = read_u32(&mut reader)?;
        if magic != BLOOM_MAGIC {
            return Err(crate::types::ForgeError::Corruption(
                "invalid bloom filter magic".to_string(),
            ));
        }
        let version = read_u32(&mut reader)?;
        if version != BLOOM_VERSION {
            return Err(crate::types::ForgeError::Corruption(
                "unsupported bloom filter version".to_string(),
            ));
        }
        let hash_count = read_u32(&mut reader)?;
        let bit_count = read_u32(&mut reader)? as usize;
        let entry_count = read_u32(&mut reader)? as usize;
        let bits_len = read_u32(&mut reader)? as usize;
        let mut bits = vec![0u8; bits_len];
        reader.read_exact(&mut bits)?;

        Ok(Self {
            bits,
            bit_count,
            hash_count,
            entry_count,
        })
    }

    fn test_bit(&self, idx: usize) -> bool {
        let byte = idx / 8;
        let bit = idx % 8;
        (self.bits[byte] & (1 << bit)) != 0
    }
}

impl BloomBuilder {
    fn new(config: BloomConfig) -> Self {
        Self {
            bits: vec![0u8; 1],
            bit_count: 8,
            hash_count: config.hash_count.max(1),
            entry_count: 0,
            bits_per_key: config.bits_per_key.max(1),
        }
    }

    /// Inserts one key into the builder.
    pub fn insert(&mut self, key: &str) {
        self.entry_count += 1;
        self.grow_if_needed();
        let (h1, h2) = hash_pair(key);
        for i in 0..self.hash_count {
            let idx = index_for_hashes(self.bit_count, h1, h2, i);
            self.set_bit(idx);
        }
    }

    /// Finishes construction and returns the immutable bloom filter.
    pub fn finish(self) -> BloomFilter {
        BloomFilter {
            bits: self.bits,
            bit_count: self.bit_count,
            hash_count: self.hash_count,
            entry_count: self.entry_count,
        }
    }

    fn grow_if_needed(&mut self) {
        let target_bits = (self.entry_count.max(1) * self.bits_per_key).max(8);
        if target_bits <= self.bit_count {
            return;
        }

        self.bit_count = target_bits;
        self.bits.resize(self.bit_count.div_ceil(8), 0);
    }

    fn set_bit(&mut self, idx: usize) {
        let byte = idx / 8;
        let bit = idx % 8;
        self.bits[byte] |= 1 << bit;
    }
}

fn index_for_hashes(bit_count: usize, h1: u64, h2: u64, i: u32) -> usize {
    let combined = h1.wrapping_add((i as u64).wrapping_mul(h2));
    (combined as usize) % bit_count
}

fn hash_pair(key: &str) -> (u64, u64) {
    let mut h1 = FNV_OFFSET_BASIS;
    let mut h2 = FNV_OFFSET_BASIS ^ 0x9e37_79b9_7f4a_7c15;
    for &b in key.as_bytes() {
        h1 ^= b as u64;
        h1 = h1.wrapping_mul(FNV_PRIME);

        h2 ^= (b as u64).wrapping_add(0x9e);
        h2 = h2.wrapping_mul(FNV_PRIME ^ 0x517c_c1b7_2722_0a95);
    }
    (h1, h2 | 1)
}
