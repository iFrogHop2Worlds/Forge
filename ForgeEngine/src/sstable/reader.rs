use std::fs::File;
use std::io::{BufReader, ErrorKind, Seek, SeekFrom};
use std::path::Path;

use crate::sstable::block::read_entry;
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result};

#[derive(Debug)]
pub struct SstableIterator {
    reader: BufReader<File>,
    current: Option<Entry>,
    done: bool,
}

impl SstableIterator {
    pub fn open(path: &Path) -> Result<Self> {
        let mut this = Self {
            reader: BufReader::new(File::open(path)?),
            current: None,
            done: false,
        };
        this.advance()?;
        Ok(this)
    }

    pub fn valid(&self) -> bool {
        self.current.is_some()
    }

    pub fn value(&self) -> &Entry {
        self.current
            .as_ref()
            .expect("invalid sstable iterator access")
    }

    pub fn next(&mut self) -> Result<()> {
        self.advance()
    }

    fn advance(&mut self) -> Result<()> {
        if self.done {
            self.current = None;
            return Ok(());
        }

        match read_entry(&mut self.reader) {
            Ok(entry) => {
                self.current = Some(entry);
            }
            Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                self.current = None;
                self.done = true;
            }
            Err(err) => return Err(err),
        }

        Ok(())
    }
}

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
