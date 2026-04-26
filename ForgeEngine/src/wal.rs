use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::types::{Entry, Result, ValueRef};
use crate::util::{decode_value, read_i32, read_u32, read_u64, write_i32, write_u32, write_u64};

#[derive(Debug)]
pub struct Wal {
    path: PathBuf,
    writer: BufWriter<File>,
    buffered_bytes: usize,
}

impl Wal {
    const FLUSH_BYTES: usize = 256 * 1024;

    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("current.wal");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;

        Ok(Self {
            path,
            writer: BufWriter::new(file),
            buffered_bytes: 0,
        })
    }

    pub fn append(&mut self, entry: &Entry) -> Result<()> {
        write_u64(&mut self.writer, entry.seq)?;
        write_u32(&mut self.writer, entry.key.len() as u32)?;
        let value_len = match &entry.value {
            ValueRef::Value(v) => v.len() as i32,
            ValueRef::Tombstone => -1,
        };
        write_i32(&mut self.writer, value_len)?;
        self.writer.write_all(entry.key.as_bytes())?;
        if let ValueRef::Value(v) = &entry.value {
            self.writer.write_all(v)?;
        }

        self.buffered_bytes += 8 + 4 + 4 + entry.key.len() + value_len.max(0) as usize;
        if self.buffered_bytes >= Self::FLUSH_BYTES {
            self.writer.flush()?;
            self.buffered_bytes = 0;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.buffered_bytes = 0;
        Ok(())
    }

    pub fn replay(path: &Path) -> Result<Vec<Entry>> {
        if !path.exists() {
            return Ok(Vec::new());
        }

        let mut reader = BufReader::new(File::open(path)?);
        let mut entries = Vec::new();

        loop {
            let seq = match read_u64(&mut reader) {
                Ok(v) => v,
                Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(e) => return Err(e),
            };

            let key_len = match read_u32(&mut reader) {
                Ok(v) => v as usize,
                Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(e) => return Err(e),
            };
            let val_len = match read_i32(&mut reader) {
                Ok(v) => v,
                Err(crate::types::ForgeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(e) => return Err(e),
            };

            let mut key = vec![0u8; key_len];
            match reader.read_exact(&mut key) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(err.into()),
            }

            let mut value_bytes = Vec::new();
            if val_len > 0 {
                value_bytes.resize(val_len as usize, 0);
                match reader.read_exact(&mut value_bytes) {
                    Ok(()) => {}
                    Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
                    Err(err) => return Err(err.into()),
                }
            }

            let value = decode_value(val_len, value_bytes)?;
            let key = String::from_utf8(key)
                .map_err(|_| crate::types::ForgeError::Corruption("non-utf8 key in wal".to_string()))?;

            entries.push(Entry { key, value, seq });
        }

        Ok(entries)
    }

    pub fn reset(&mut self) -> Result<()> {
        self.flush()?;
        let mut file = OpenOptions::new().write(true).open(&self.path)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;

        self.writer = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(&self.path)?,
        );
        self.buffered_bytes = 0;

        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
