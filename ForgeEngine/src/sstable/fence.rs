use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use crate::types::Result;
use crate::util::{read_u32, write_u32};

#[derive(Debug, Clone)]
pub struct KeyRangeFence {
    pub min_key: String,
    pub max_key: String,
}

impl KeyRangeFence {
    pub fn new(min_key: String, max_key: String) -> Self {
        Self { min_key, max_key }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.min_key.as_str() <= key && key <= self.max_key.as_str()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        write_u32(&mut writer, self.min_key.len() as u32)?;
        writer.write_all(self.min_key.as_bytes())?;
        write_u32(&mut writer, self.max_key.len() as u32)?;
        writer.write_all(self.max_key.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let min_len = read_u32(&mut reader)? as usize;
        let mut min_key = vec![0u8; min_len];
        reader.read_exact(&mut min_key)?;

        let max_len = read_u32(&mut reader)? as usize;
        let mut max_key = vec![0u8; max_len];
        reader.read_exact(&mut max_key)?;

        Ok(Self {
            min_key: String::from_utf8(min_key).map_err(|_| {
                crate::types::ForgeError::Corruption("non-utf8 min key in fence".to_string())
            })?,
            max_key: String::from_utf8(max_key).map_err(|_| {
                crate::types::ForgeError::Corruption("non-utf8 max key in fence".to_string())
            })?,
        })
    }
}
