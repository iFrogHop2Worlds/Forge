use crate::types::{Entry, Result, ValueRef};
use crate::util::{decode_value, encode_value_len, read_i32, read_u32, read_u64, write_i32, write_u32, write_u64};
use std::io::{Read, Write};

pub fn write_entry(mut w: impl Write, entry: &Entry) -> Result<usize> {
    write_u64(&mut w, entry.seq)?;
    write_u32(&mut w, entry.key.len() as u32)?;
    let val_len = encode_value_len(&entry.value);
    write_i32(&mut w, val_len)?;
    w.write_all(entry.key.as_bytes())?;
    if let ValueRef::Value(v) = &entry.value {
        w.write_all(v)?;
    }

    let bytes = 8 + 4 + 4 + entry.key.len() + if val_len > 0 { val_len as usize } else { 0 };
    Ok(bytes)
}

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
