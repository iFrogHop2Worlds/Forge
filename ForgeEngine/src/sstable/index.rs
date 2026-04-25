use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use crate::types::Result;
use crate::util::{read_u32, read_u64, write_u32, write_u64};

#[derive(Debug, Clone)]
pub struct SparseIndex {
    pub entries: Vec<(String, u64)>,
}

impl SparseIndex {
    pub fn new(entries: Vec<(String, u64)>) -> Self {
        Self { entries }
    }

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
            let key = String::from_utf8(key)
                .map_err(|_| crate::types::ForgeError::Corruption("non-utf8 key in index".to_string()))?;
            entries.push((key, offset));
        }

        Ok(Self { entries })
    }

    pub fn floor_offset_for(&self, key: &str) -> u64 {
        let mut best = 0;
        for (k, offset) in &self.entries {
            if k.as_str() <= key {
                best = *offset;
            } else {
                break;
            }
        }
        best
    }
}
