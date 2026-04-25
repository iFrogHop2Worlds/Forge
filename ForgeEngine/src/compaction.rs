use std::collections::BTreeMap;

use crate::types::ValueRef;

pub fn merge_for_compaction<I>(iter: I) -> BTreeMap<String, (u64, ValueRef)>
where
    I: IntoIterator<Item = (String, u64, ValueRef)>,
{
    let mut merged = BTreeMap::new();
    for (key, seq, value) in iter {
        let update = match merged.get(&key) {
            Some((existing_seq, _)) => seq > *existing_seq,
            None => true,
        };
        if update {
            merged.insert(key, (seq, value));
        }
    }
    merged
}
