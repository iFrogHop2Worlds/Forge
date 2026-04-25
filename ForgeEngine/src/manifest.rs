use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::types::{ForgeError, Result};

#[derive(Debug, Clone)]
pub struct TableMeta {
    pub id: u64,
    pub level: u8,
    pub data_path: PathBuf,
    pub index_path: PathBuf,
}

impl TableMeta {
    pub fn new(base_dir: &Path, id: u64, level: u8) -> Self {
        Self {
            id,
            level,
            data_path: base_dir.join(format!("L{level}_{id}.sst")),
            index_path: base_dir.join(format!("L{level}_{id}.idx")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub next_table_id: u64,
    pub levels: Vec<Vec<TableMeta>>,
}

impl Manifest {
    pub fn empty(level_count: usize) -> Self {
        Self {
            next_table_id: 1,
            levels: vec![Vec::new(); level_count],
        }
    }

    pub fn load(base_dir: &Path, level_count: usize) -> Result<Option<Self>> {
        let path = base_dir.join("MANIFEST");
        if !path.exists() {
            return Ok(None);
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut manifest = Self::empty(level_count);

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let mut parts = line.split_whitespace();
            let tag = parts.next().unwrap_or_default();
            match tag {
                "NEXT" => {
                    let id = parts
                        .next()
                        .ok_or_else(|| ForgeError::Corruption("manifest NEXT missing id".to_string()))?
                        .parse::<u64>()
                        .map_err(|_| ForgeError::Corruption("manifest NEXT invalid id".to_string()))?;
                    manifest.next_table_id = id.max(1);
                }
                "TABLE" => {
                    let level = parts
                        .next()
                        .ok_or_else(|| ForgeError::Corruption("manifest TABLE missing level".to_string()))?
                        .parse::<usize>()
                        .map_err(|_| ForgeError::Corruption("manifest TABLE invalid level".to_string()))?;
                    let id = parts
                        .next()
                        .ok_or_else(|| ForgeError::Corruption("manifest TABLE missing id".to_string()))?
                        .parse::<u64>()
                        .map_err(|_| ForgeError::Corruption("manifest TABLE invalid id".to_string()))?;

                    if level >= level_count {
                        return Err(ForgeError::Corruption("manifest TABLE level out of range".to_string()));
                    }

                    let table = TableMeta::new(base_dir, id, level as u8);
                    manifest.levels[level].push(table);
                }
                _ => {
                    return Err(ForgeError::Corruption(format!("manifest unknown tag: {tag}")));
                }
            }
        }

        for level in &mut manifest.levels {
            level.sort_by(|a, b| b.id.cmp(&a.id));
        }

        if manifest.next_table_id == 0 {
            manifest.next_table_id = 1;
        }

        Ok(Some(manifest))
    }

    pub fn save(&self, base_dir: &Path) -> Result<()> {
        fs::create_dir_all(base_dir)?;
        let tmp = base_dir.join("MANIFEST.tmp");
        let dst = base_dir.join("MANIFEST");

        {
            let mut writer = BufWriter::new(File::create(&tmp)?);
            writeln!(writer, "NEXT {}", self.next_table_id)?;

            for level in &self.levels {
                for t in level {
                    writeln!(writer, "TABLE {} {}", t.level, t.id)?;
                }
            }
            writer.flush()?;
        }

        if dst.exists() {
            fs::remove_file(&dst)?;
        }
        fs::rename(tmp, dst)?;
        Ok(())
    }
}
