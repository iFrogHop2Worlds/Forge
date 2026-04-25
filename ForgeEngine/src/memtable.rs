use std::collections::BTreeMap;

use crate::types::ValueRef;

#[derive(Debug, Default)]
pub struct MemTable {
    map: BTreeMap<String, (u64, ValueRef)>,
    approx_bytes: usize,
}

impl MemTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, seq: u64, key: String, value: ValueRef) {
        let new_size = key.len()
            + match &value {
                ValueRef::Value(v) => v.len(),
                ValueRef::Tombstone => 0,
            };

        if let Some((_, old_value)) = self.map.insert(key.clone(), (seq, value)) {
            self.approx_bytes = self.approx_bytes.saturating_sub(
                key.len()
                    + match old_value {
                        ValueRef::Value(v) => v.len(),
                        ValueRef::Tombstone => 0,
                    },
            );
        }

        self.approx_bytes += new_size;
    }

    pub fn get(&self, key: &str) -> Option<(u64, ValueRef)> {
        self.map.get(key).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.approx_bytes = 0;
    }

    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    pub fn iter_sorted(&self) -> impl Iterator<Item = (String, u64, ValueRef)> + '_ {
        self.map
            .iter()
            .map(|(k, (seq, value))| (k.clone(), *seq, value.clone()))
    }
}
