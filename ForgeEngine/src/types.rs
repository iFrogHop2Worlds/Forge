use std::fmt::{Display, Formatter};
use std::io;

/// Represents the stored value state for a database entry.
///
/// # Variants
///
/// * `Value` - Contains the raw bytes associated with a key.
/// * `Tombstone` - Marks a key as deleted while preserving delete ordering for
///   WAL replay and compaction.
///
/// # Behavior
///
/// - Values are stored as byte vectors so callers can persist arbitrary binary data.
/// - Tombstones are written through the same entry path as values and are resolved
///   during reads and compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueRef {
    Value(Vec<u8>),
    Tombstone,
}

/// Represents a single versioned key-value record in the storage engine.
///
/// # Fields
///
/// * `key` - The string key used to order and locate the record.
/// * `value` - The value payload or tombstone marker associated with the key.
/// * `seq` - The sequence number assigned to the write. Higher sequence numbers
///   represent newer versions of the same key.
///
/// # Notes
///
/// - Entries are written to the WAL and SSTable files using the same logical shape.
/// - SSTable operations assume entries are sorted by key before they are written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: ValueRef,
    pub seq: u64,
}

/// Represents errors returned by the Forge storage engine.
///
/// # Variants
///
/// * `Io` - Wraps filesystem and stream errors returned by the standard library.
/// * `Corruption` - Indicates malformed persisted data such as invalid UTF-8 keys
///   or invalid encoded value lengths.
///
/// # Behavior
///
/// - I/O errors are converted automatically through the `From<io::Error>`
///   implementation.
/// - Corruption errors are created explicitly when persisted data cannot be decoded
///   into the expected engine format.
#[derive(Debug)]
pub enum ForgeError {
    Io(io::Error),
    Corruption(String),
}

impl Display for ForgeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::Corruption(msg) => write!(f, "data corruption: {msg}"),
        }
    }
}

impl std::error::Error for ForgeError {}

impl From<io::Error> for ForgeError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Convenience result type used by Forge storage operations.
///
/// # Type Parameters
///
/// * `T` - The success value returned by the operation.
///
/// # Returns
///
/// * `Ok(T)` when the operation succeeds.
/// * `Err(ForgeError)` when an I/O or corruption error occurs.
pub type Result<T> = std::result::Result<T, ForgeError>;
