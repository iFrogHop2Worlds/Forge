use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::types::{Entry, Result, ValueRef};
use crate::util::{decode_value, read_i32, read_u32, read_u64, write_i32, write_u32, write_u64};

/// Represents the write-ahead log used to persist recent database mutations.
///
/// # Fields
///
/// * `path` - The file path of the active WAL file.
/// * `writer` - A buffered writer used to append encoded entries to the WAL.
/// * `buffered_bytes` - The approximate number of bytes written since the last
///   flush through this `Wal` instance.
///
/// # Behavior
///
/// - Entries are appended before they are applied to the in-memory table.
/// - WAL replay reconstructs entries that were persisted before a shutdown or crash.
/// - The WAL can be reset after a memtable flush because the flushed entries are
///   represented by SSTables.
#[derive(Debug)]
pub struct Wal {
    path: PathBuf,
    writer: BufWriter<File>,
    buffered_bytes: usize,
}

impl Wal {
    const FLUSH_BYTES: usize = 124 * 1024 * 1024;

    /// Opens the active WAL file in the specified database directory.
    ///
    /// # Parameters
    /// - `dir`: The database directory containing the active `current.wal` file.
    ///
    /// # Returns
    /// - `Result<Self>`: Returns an opened `Wal` on success, or an error if the
    ///   WAL file cannot be created or opened.
    ///
    /// # Behavior
    /// - Uses `current.wal` as the active WAL filename.
    /// - Creates the WAL file if it does not already exist.
    /// - Opens the file in append mode so new entries are added to the end.
    ///
    /// # Errors
    /// - Returns an error if the WAL file cannot be created or opened.
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

    /// Appends an entry to the write-ahead log.
    ///
    /// # Parameters
    /// - `entry`: The `Entry` object containing the sequence number, key, and value
    ///   or tombstone marker to persist.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` when the entry is appended successfully,
    ///   or an error if writing fails.
    ///
    /// # Behavior
    /// - Writes the sequence number, key length, and encoded value length.
    /// - Writes the key bytes and, for value entries, the value bytes.
    /// - Tracks buffered bytes and flushes automatically when the flush threshold
    ///   is reached.
    ///
    /// # Errors
    /// - Returns an error if any WAL field or byte payload cannot be written.
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

    /// Flushes buffered WAL data to the operating system.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` when buffered data is flushed, or an error
    ///   if flushing fails.
    ///
    /// # Behavior
    /// - Flushes the buffered writer.
    /// - Resets the buffered byte counter to zero after a successful flush.
    ///
    /// # Errors
    /// - Returns an error if the underlying writer cannot flush its buffer.
    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        self.buffered_bytes = 0;
        Ok(())
    }

    /// Replays entries from a WAL file.
    ///
    /// # Parameters
    /// - `path`: The path to the WAL file to replay.
    ///
    /// # Returns
    /// - `Result<Vec<Entry>>`: Returns the decoded entries in WAL order on success,
    ///   or an error if the WAL cannot be opened or decoded.
    ///
    /// # Behavior
    /// - Returns an empty vector when the WAL file does not exist.
    /// - Reads entries sequentially from the WAL file.
    /// - Treats a partial trailing entry as end-of-log and stops replaying.
    ///
    /// # Errors
    /// - Returns an error if the WAL file cannot be opened or read.
    /// - Returns a corruption error if a key is not valid UTF-8 or a value cannot
    ///   be decoded.
    pub fn replay(path: &Path) -> Result<Vec<Entry>> {
        if !path.exists() {
            return Ok(Vec::new());
        }

        let mut reader = BufReader::new(File::open(path)?);
        let mut entries = Vec::new();

        loop {
            let seq = match read_u64(&mut reader) {
                Ok(v) => v,
                Err(crate::types::ForgeError::Io(err))
                    if err.kind() == ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(e),
            };

            let key_len = match read_u32(&mut reader) {
                Ok(v) => v as usize,
                Err(crate::types::ForgeError::Io(err))
                    if err.kind() == ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(e),
            };
            let val_len = match read_i32(&mut reader) {
                Ok(v) => v,
                Err(crate::types::ForgeError::Io(err))
                    if err.kind() == ErrorKind::UnexpectedEof =>
                {
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
            let key = String::from_utf8(key).map_err(|_| {
                crate::types::ForgeError::Corruption("non-utf8 key in wal".to_string())
            })?;

            entries.push(Entry { key, value, seq });
        }

        Ok(entries)
    }

    /// Clears the active WAL after its entries have been persisted elsewhere.
    ///
    /// # Returns
    /// - `Result<()>`: Returns `Ok(())` when the WAL is truncated and reopened,
    ///   or an error if any filesystem operation fails.
    ///
    /// # Behavior
    /// - Flushes any buffered WAL data before truncation.
    /// - Truncates the active WAL file to zero bytes.
    /// - Reopens the WAL in append mode for future writes.
    /// - Resets the buffered byte counter.
    ///
    /// # Errors
    /// - Returns an error if flushing, truncating, seeking, or reopening the WAL fails.
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

    /// Returns the filesystem path of the active WAL.
    ///
    /// # Returns
    /// - `&Path`: A shared reference to the active WAL path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}
