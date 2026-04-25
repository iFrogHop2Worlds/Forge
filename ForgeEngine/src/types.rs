use std::fmt::{Display, Formatter};
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueRef {
    Value(Vec<u8>),
    Tombstone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: ValueRef,
    pub seq: u64,
}

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

pub type Result<T> = std::result::Result<T, ForgeError>;
