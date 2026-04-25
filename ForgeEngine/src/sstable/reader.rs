use std::fs::File;
use std::io::{BufReader, ErrorKind, Seek, SeekFrom};
use std::path::Path;

use crate::sstable::block::read_entry;
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result};

pub fn read_all(path: &Path) -> Result<Vec<Entry>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut out = Vec::new();

    loop {
        match read_entry(&mut reader) {
            Ok(entry) => out.push(entry),
            Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        }
    }

    Ok(out)
}

pub fn get(path: &Path, index_path: &Path, key: &str) -> Result<Option<Entry>> {
    let index = SparseIndex::load(index_path)?;
    let mut reader = BufReader::new(File::open(path)?);
    let start = index.floor_offset_for(key);
    reader.seek(SeekFrom::Start(start))?;

    loop {
        match read_entry(&mut reader) {
            Ok(entry) => {
                if entry.key == key {
                    return Ok(Some(entry));
                }
                if entry.key.as_str() > key {
                    return Ok(None);
                }
            }
            Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                return Ok(None)
            }
            Err(err) => return Err(err),
        }
    }
}
