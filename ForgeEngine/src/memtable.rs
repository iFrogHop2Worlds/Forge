use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::types::ValueRef;

/// Represents the mutable in-memory table used for recent database writes.
///
/// # Fields
///
/// * `map` - A `BTreeMap` keyed by user key. Each value contains the most recent
///   sequence number and value state for that key.
/// * `approx_bytes` - An approximate byte count of keys and values currently held
///   by the memtable.
///
/// # Behavior
///
/// - Entries are ordered by key so they can be flushed directly into sorted SSTable
///   entries.
/// - Inserts replace the previous value for the same key and update the approximate
///   byte count.
/// - Tombstones are retained in memory as delete markers until the memtable is
///   flushed.
#[derive(Debug, Default)]
pub struct MemTable {
    map: BTreeMap<String, (u64, ValueRef)>,
    approx_bytes: usize,
}

impl MemTable {
    /// Creates an empty memtable.
    ///
    /// # Returns
    /// - `Self`: Returns a memtable with no entries and an approximate byte count
    ///   of zero.
    ///
    /// # Behavior
    /// - Uses the default `BTreeMap` ordering for key storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces a key in the memtable.
    ///
    /// # Parameters
    /// - `seq`: The sequence number assigned to the write.
    /// - `key`: The key to insert or replace.
    /// - `value`: The value bytes or tombstone marker associated with the key.
    ///
    /// # Behavior
    /// - Stores the sequence number and value state under the provided key.
    /// - Removes the previous approximate byte contribution when replacing an
    ///   existing key.
    /// - Adds the new key and value contribution to the approximate byte count.
    ///
    /// # Notes
    /// - Tombstones contribute key bytes but no value bytes to the approximate size.
    pub fn insert(&mut self, seq: u64, key: String, value: ValueRef) {
        let new_size = key.len()
            + match &value {
                ValueRef::Value(v) => v.len(),
                ValueRef::Tombstone => 0,
            };

        match self.map.entry(key) {
            Entry::Occupied(mut occupied) => {
                let old_size = occupied.key().len()
                    + match &occupied.get().1 {
                        ValueRef::Value(v) => v.len(),
                        ValueRef::Tombstone => 0,
                    };
                self.approx_bytes = self.approx_bytes.saturating_sub(old_size);
                occupied.insert((seq, value));
            }
            Entry::Vacant(vacant) => {
                vacant.insert((seq, value));
            }
        };

        self.approx_bytes += new_size;
    }

    /// Retrieves the current entry state for a key.
    ///
    /// # Parameters
    /// - `key`: The key to search for in the memtable.
    ///
    /// # Returns
    /// - `Option<(u64, ValueRef)>`: Returns the sequence number and value state
    ///   when the key exists, or `None` when it is not present.
    ///
    /// # Behavior
    /// - Clones the stored value state so callers can use the result without
    ///   borrowing the memtable.
    pub fn get(&self, key: &str) -> Option<(u64, ValueRef)> {
        self.map.get(key).cloned()
    }

    /// Indicates whether the memtable contains any entries.
    ///
    /// # Returns
    /// - `bool`: Returns `true` when the memtable has no entries, or `false` when
    ///   at least one key is present.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Removes all entries from the memtable.
    ///
    /// # Behavior
    /// - Clears the underlying key-value map.
    /// - Resets the approximate byte count to zero.
    pub fn clear(&mut self) {
        self.map.clear();
        self.approx_bytes = 0;
    }

    /// Returns the approximate byte usage of the memtable.
    ///
    /// # Returns
    /// - `usize`: The approximate number of key and value bytes tracked by the
    ///   memtable.
    ///
    /// # Notes
    /// - The value is used as a flush threshold signal and does not include every
    ///   allocation or data structure overhead byte.
    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    /// Iterates over memtable entries in sorted key order.
    ///
    /// # Returns
    /// - `impl Iterator<Item = (String, u64, ValueRef)>`: An iterator yielding cloned
    ///   keys, sequence numbers, and value states in ascending key order.
    ///
    /// # Behavior
    /// - Uses the natural ordering of the underlying `BTreeMap`.
    /// - Clones keys and value states so the iterator output can be used where
    ///   owned entries are required.
    ///
    /// # Notes
    /// - Flush paths that can write borrowed entries should prefer `iter_sorted_ref`
    ///   to avoid cloning keys and values.
    pub fn iter_sorted(&self) -> impl Iterator<Item = (String, u64, ValueRef)> + '_ {
        self.map
            .iter()
            .map(|(k, (seq, value))| (k.clone(), *seq, value.clone()))
    }

    /// Iterates over borrowed memtable entries in sorted key order.
    ///
    /// # Returns
    /// - `impl Iterator<Item = (&String, u64, &ValueRef)>`: An iterator yielding
    ///   borrowed keys, sequence numbers, and borrowed value states in ascending
    ///   key order.
    ///
    /// # Behavior
    /// - Uses the natural ordering of the underlying `BTreeMap`.
    /// - Avoids cloning keys and values while the memtable is being flushed.
    ///
    /// # Notes
    /// - This iterator is intended for streaming encoders that do not need owned
    ///   `Entry` values.
    pub fn iter_sorted_ref(&self) -> impl Iterator<Item = (&String, u64, &ValueRef)> + '_ {
        self.map.iter().map(|(k, (seq, value))| (k, *seq, value))
    }
}
