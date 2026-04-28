use std::io::{Read, Write};

use crate::types::{ForgeError, Result, ValueRef};

pub fn write_u32(mut w: impl Write, v: u32) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}

pub fn write_u64(mut w: impl Write, v: u64) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}

pub fn write_i32(mut w: impl Write, v: i32) -> Result<()> {
    w.write_all(&v.to_le_bytes())?;
    Ok(())
}

pub fn read_u32(mut r: impl Read) -> Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

pub fn read_u64(mut r: impl Read) -> Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

pub fn read_i32(mut r: impl Read) -> Result<i32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(i32::from_le_bytes(buf))
}

pub fn encode_value_len(value: &ValueRef) -> i32 {
    match value {
        ValueRef::Value(v) => v.len() as i32,
        ValueRef::Tombstone => -1,
    }
}

pub fn decode_value(len: i32, bytes: Vec<u8>) -> Result<ValueRef> {
    if len == -1 {
        if !bytes.is_empty() {
            return Err(ForgeError::Corruption(
                "tombstone record had payload bytes".to_string(),
            ));
        }
        return Ok(ValueRef::Tombstone);
    }

    if len < -1 {
        return Err(ForgeError::Corruption(
            "invalid negative value length".to_string(),
        ));
    }

    Ok(ValueRef::Value(bytes))
}
