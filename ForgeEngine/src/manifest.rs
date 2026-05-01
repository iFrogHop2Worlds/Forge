use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::types::{ForgeError, Result};

/// Represents the metadata for a table, including its unique identifier,
/// storage paths, and hierarchical level.
///
/// # Fields
///
/// * `id` - A 64-bit unsigned integer that uniquely identifies the table.
/// * `level` - An 8-bit unsigned integer representing the table's hierarchical
///   level, which may be used to organize tables within a multilevel structure.
/// * `data_path` - A `PathBuf` that specifies the file system path to the table's data.
/// * `index_path` - A `PathBuf` that specifies the file system path to the table's index.
/// * `bloom_path` - A `PathBuf` that specifies the file system path to the table's bloom filter.
///
/// This struct derives the following traits:
/// * `Debug` - Enables formatting the struct using the `{:?}` formatter.
/// * `Clone` - Allows for the creation of deep copies of the struct.
#[derive(Debug, Clone)]
pub struct TableMeta {
    pub id: u64,
    pub level: u8,
    pub data_path: PathBuf,
    pub index_path: PathBuf,
    pub bloom_path: PathBuf,
}

impl TableMeta {
    /// Creates table metadata for a specific table ID and level.
    ///
    /// # Parameters
    /// - `base_dir`: The database directory where the table files are stored.
    /// - `id`: The unique table identifier.
    /// - `level`: The level where the table belongs in the LSM-tree.
    ///
    /// # Returns
    /// - `Self`: Returns a `TableMeta` with the data and index paths derived from
    ///   the provided base directory, level, and table ID.
    ///
    /// # Behavior
    /// - Creates an SSTable data path using the `L<level>_<id>.sst` naming format.
    /// - Creates a sparse index path using the `L<level>_<id>.idx` naming format.
    /// - Creates a bloom filter path using the `L<level>_<id>.bf` naming format.
    pub fn new(base_dir: &Path, id: u64, level: u8) -> Self {
        Self {
            id,
            level,
            data_path: base_dir.join(format!("L{level}_{id}.sst")),
            index_path: base_dir.join(format!("L{level}_{id}.idx")),
            bloom_path: base_dir.join(format!("L{level}_{id}.bf")),
        }
    }
}

/// Represents the persisted table layout for the database.
///
/// # Fields
///
/// * `next_table_id` - The next table identifier to assign when a new SSTable is
///   created.
/// * `levels` - The list of table metadata grouped by level in the LSM-tree.
///
/// # Behavior
///
/// - The manifest records enough metadata to reopen the database without scanning
///   every table file during normal startup.
/// - The manifest file stores the next table ID and the table IDs present at each
///   level.
///
/// # Notes
///
/// - Table metadata is sorted by descending table ID after loading.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub next_table_id: u64,
    pub levels: Vec<Vec<TableMeta>>,
}

impl Manifest {
    /// Creates an empty manifest with the requested number of levels.
    ///
    /// # Parameters
    /// - `level_count`: The number of levels to allocate in the manifest.
    ///
    /// # Returns
    /// - `Self`: Returns a manifest with no tables and a `next_table_id` of `1`.
    ///
    /// # Behavior
    /// - Allocates one empty table list for each level.
    /// - Initializes table ID assignment at `1`.
    pub fn empty(level_count: usize) -> Self {
        Self {
            next_table_id: 1,
            levels: vec![Vec::new(); level_count],
        }
    }

    /// Loads the manifest file from a database directory.
    ///
    /// # Parameters
    /// - `base_dir`: The database directory containing the `MANIFEST` file.
    /// - `level_count`: The number of levels expected by the database.
    ///
    /// # Returns
    /// - `Result<Option<Self>>`: Returns `Ok(Some(Manifest))` when a manifest is
    ///   found and decoded, `Ok(None)` when no manifest exists, or an error if the
    ///   manifest cannot be read or parsed.
    ///
    /// # Behavior
    /// - Reads the `MANIFEST` file line by line.
    /// - Parses `NEXT <id>` records to restore the next table ID.
    /// - Parses `TABLE <level> <id>` records to restore table metadata.
    /// - Sorts each level by descending table ID after loading.
    ///
    /// # Errors
    /// - Returns an error if the manifest file cannot be opened or read.
    /// - Returns a corruption error if a manifest record is missing required fields,
    ///   contains invalid numbers, uses an unknown tag, or references an out-of-range
    ///   level.
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
                        .ok_or_else(|| {
                            ForgeError::Corruption("manifest NEXT missing id".to_string())
                        })?
                        .parse::<u64>()
                        .map_err(|_| {
                            ForgeError::Corruption("manifest NEXT invalid id".to_string())
                        })?;
                    manifest.next_table_id = id.max(1);
                }
                "TABLE" => {
                    let level = parts
                        .next()
                        .ok_or_else(|| {
                            ForgeError::Corruption("manifest TABLE missing level".to_string())
                        })?
                        .parse::<usize>()
                        .map_err(|_| {
                            ForgeError::Corruption("manifest TABLE invalid level".to_string())
                        })?;
                    let id = parts
                        .next()
                        .ok_or_else(|| {
                            ForgeError::Corruption("manifest TABLE missing id".to_string())
                        })?
                        .parse::<u64>()
                        .map_err(|_| {
                            ForgeError::Corruption("manifest TABLE invalid id".to_string())
                        })?;

                    if level >= level_count {
                        return Err(ForgeError::Corruption(
                            "manifest TABLE level out of range".to_string(),
                        ));
                    }

                    let table = TableMeta::new(base_dir, id, level as u8);
                    manifest.levels[level].push(table);
                }
                _ => {
                    return Err(ForgeError::Corruption(format!(
                        "manifest unknown tag: {tag}"
                    )));
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

    /// Saves the manifest to a database directory.
    ///
    /// # Parameters
    /// - `base_dir`: The database directory where the `MANIFEST` file will be stored.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` when the manifest is saved, or an error if
    ///   the file cannot be written or replaced.
    ///
    /// # Behavior
    /// - Ensures the database directory exists.
    /// - Writes the manifest contents to `MANIFEST.tmp`.
    /// - Replaces the existing `MANIFEST` file with the completed temporary file.
    /// - Writes one `NEXT` record followed by one `TABLE` record per table.
    ///
    /// # Errors
    /// - Returns an error if the directory cannot be created.
    /// - Returns an error if the temporary manifest cannot be created, written, or
    ///   flushed.
    /// - Returns an error if the old manifest cannot be removed or the temporary file
    ///   cannot be renamed into place.
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
