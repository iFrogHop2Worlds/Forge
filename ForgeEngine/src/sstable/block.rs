use crate::types::{Entry, Result, ValueRef};
use crate::util::{
    decode_value, encode_value_len, read_i32, read_u32, read_u64, write_i32, write_u32, write_u64,
};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

/// Writes a single SSTable entry to the provided output stream.
///
/// # Parameters
/// - `w`: The output stream that receives the encoded entry bytes.
/// - `entry`: The `Entry` object containing the sequence number, key, and value
///   or tombstone marker to write.
///
/// # Returns
/// - `Result<usize>`: Returns the number of bytes written on success, or an error
///   if writing to the stream fails.
///
/// # Behavior
/// - Writes the entry sequence number as a `u64`.
/// - Writes the key length as a `u32`.
/// - Writes the encoded value length as an `i32`, where tombstones are represented
///   by a negative length.
/// - Writes the UTF-8 key bytes followed by value bytes when the entry contains a value.
///
/// # Errors
/// - Returns an error if any field cannot be written to the output stream.
///
/// # Notes
/// - This function does not sort or validate entry ordering. Callers are responsible
///   for writing entries in SSTable order.
pub fn write_entry(mut w: impl Write, entry: &Entry) -> Result<usize> {
    write_entry_ref(&mut w, entry.seq, &entry.key, &entry.value)
}

/// Writes a single borrowed SSTable entry to the provided output stream.
///
/// # Parameters
/// - `w`: The output stream that receives the encoded entry bytes.
/// - `seq`: The sequence number for the entry.
/// - `key`: The borrowed key string to write.
/// - `value`: The borrowed value or tombstone marker to write.
///
/// # Returns
/// - `Result<usize>`: Returns the number of bytes written on success, or an error
///   if writing to the stream fails.
///
/// # Behavior
/// - Writes the same binary entry format as `write_entry`.
/// - Avoids constructing or cloning an owned `Entry` when the caller already has
///   borrowed entry fields.
///
/// # Errors
/// - Returns an error if any field cannot be written to the output stream.
pub fn write_entry_ref(mut w: impl Write, seq: u64, key: &str, value: &ValueRef) -> Result<usize> {
    write_u64(&mut w, seq)?;
    write_u32(&mut w, key.len() as u32)?;
    let val_len = encode_value_len(value);
    write_i32(&mut w, val_len)?;
    w.write_all(key.as_bytes())?;
    if let ValueRef::Value(v) = value {
        w.write_all(v)?;
    }

    let bytes = 8 + 4 + 4 + key.len() + if val_len > 0 { val_len as usize } else { 0 };
    Ok(bytes)
}

/// Reads a single SSTable entry from the provided input stream.
///
/// # Parameters
/// - `r`: The input stream containing an encoded SSTable entry.
///
/// # Returns
/// - `Result<Entry>`: Returns the decoded `Entry` on success, or an error if the
///   entry cannot be read or decoded.
///
/// # Behavior
/// - Reads the sequence number, key length, and encoded value length from the stream.
/// - Reads the key bytes and converts them into a UTF-8 `String`.
/// - Reads value bytes when the encoded value length is positive.
/// - Decodes the value bytes into either `ValueRef::Value` or `ValueRef::Tombstone`.
///
/// # Errors
/// - Returns an I/O error if the stream ends before a complete entry is read.
/// - Returns a corruption error if the key is not valid UTF-8 or the value length
///   cannot be decoded.
pub fn read_entry(mut r: impl Read) -> Result<Entry> {
    let seq = read_u64(&mut r)?;
    let key_len = read_u32(&mut r)? as usize;
    let val_len = read_i32(&mut r)?;

    let mut key_bytes = vec![0u8; key_len];
    r.read_exact(&mut key_bytes)?;

    let mut value_bytes = Vec::new();
    if val_len > 0 {
        value_bytes.resize(val_len as usize, 0);
        r.read_exact(&mut value_bytes)?;
    }

    let key = String::from_utf8(key_bytes)
        .map_err(|_| crate::types::ForgeError::Corruption("non-utf8 key in sstable".to_string()))?;

    Ok(Entry {
        key,
        value: decode_value(val_len, value_bytes)?,
        seq,
    })
}

/// Searches a raw encoded SSTable block and returns the matched entry value.
pub fn find_entry_in_block(block: &[u8], key: &str) -> Result<Option<Entry>> {
    let mut cursor = Cursor::new(block);

    while (cursor.position() as usize) < block.len() {
        let seq = read_u64(&mut cursor)?;
        let key_len = read_u32(&mut cursor)? as usize;
        let val_len = read_i32(&mut cursor)?;

        let mut key_bytes = vec![0u8; key_len];
        cursor.read_exact(&mut key_bytes)?;
        let entry_key = std::str::from_utf8(&key_bytes).map_err(|_| {
            crate::types::ForgeError::Corruption("non-utf8 key in sstable".to_string())
        })?;

        let value_len = val_len.max(0) as usize;
        if entry_key == key {
            let mut value_bytes = vec![0u8; value_len];
            if value_len > 0 {
                cursor.read_exact(&mut value_bytes)?;
            }
            return Ok(Some(Entry {
                key: entry_key.to_string(),
                value: decode_value(val_len, value_bytes)?,
                seq,
            }));
        }

        if entry_key > key {
            return Ok(None);
        }

        if value_len > 0 {
            let next_pos = cursor.position() + value_len as u64;
            cursor.set_position(next_pos);
        }
    }

    Ok(None)
}

/// Searches an encoded SSTable stream from its current position without decoding
/// every full entry value.
pub fn find_entry_in_stream(mut r: impl Read + Seek, key: &str) -> Result<Option<Entry>> {
    loop {
        let seq = match read_u64(&mut r) {
            Ok(v) => v,
            Err(crate::types::ForgeError::Io(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };

        let key_len = match read_u32(&mut r) {
            Ok(v) => v as usize,
            Err(crate::types::ForgeError::Io(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        let val_len = match read_i32(&mut r) {
            Ok(v) => v,
            Err(crate::types::ForgeError::Io(err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };

        let mut key_bytes = vec![0u8; key_len];
        match r.read_exact(&mut key_bytes) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(err.into()),
        }

        let entry_key = std::str::from_utf8(&key_bytes).map_err(|_| {
            crate::types::ForgeError::Corruption("non-utf8 key in sstable".to_string())
        })?;
        let value_len = val_len.max(0) as usize;

        if entry_key == key {
            let mut value_bytes = vec![0u8; value_len];
            if value_len > 0 {
                r.read_exact(&mut value_bytes)?;
            }
            return Ok(Some(Entry {
                key: entry_key.to_string(),
                value: decode_value(val_len, value_bytes)?,
                seq,
            }));
        }

        if entry_key > key {
            return Ok(None);
        }

        if value_len > 0 {
            r.seek(SeekFrom::Current(value_len as i64))?;
        }
    }
}
