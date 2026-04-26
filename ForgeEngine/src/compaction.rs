use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::path::Path;

use crate::sstable::reader::SstableIterator;
use crate::types::Entry;

pub trait DbIterator {
    fn valid(&self) -> bool;
    fn key(&self) -> &str;
    fn value(&self) -> &Entry;
    fn next(&mut self);
}

#[derive(Debug)]
pub struct TableIterator {
    inner: SstableIterator,
}

impl TableIterator {
    pub fn open(path: &Path) -> crate::types::Result<Self> {
        Ok(Self {
            inner: SstableIterator::open(path)?,
        })
    }
}

impl DbIterator for TableIterator {
    fn valid(&self) -> bool {
        self.inner.valid()
    }

    fn key(&self) -> &str {
        &self.inner.value().key
    }

    fn value(&self) -> &Entry {
        self.inner.value()
    }

    fn next(&mut self) {
        self.inner
            .next()
            .expect("failed to advance sstable table iterator");
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct HeapItem {
    key: String,
    seq: u64,
    iter_index: usize,
}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering so BinaryHeap behaves like min-heap by key, then max by seq.
        other
            .key
            .cmp(&self.key)
            .then_with(|| self.seq.cmp(&other.seq))
            .then_with(|| other.iter_index.cmp(&self.iter_index))
    }
}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct MergeIterator {
    iters: Vec<Box<dyn DbIterator>>,
    heap: BinaryHeap<HeapItem>,
    current: Option<Entry>,
}

impl MergeIterator {
    pub fn new(iters: Vec<Box<dyn DbIterator>>) -> Self {
        let mut this = Self {
            iters,
            heap: BinaryHeap::new(),
            current: None,
        };

        for idx in 0..this.iters.len() {
            this.push_if_valid(idx);
        }

        this.advance();
        this
    }

    fn push_if_valid(&mut self, idx: usize) {
        let iter = &self.iters[idx];
        if iter.valid() {
            self.heap.push(HeapItem {
                key: iter.key().to_string(),
                seq: iter.value().seq,
                iter_index: idx,
            });
        }
    }

    fn advance(&mut self) {
        self.current = None;

        if let Some(item) = self.heap.pop() {
            let iter = &mut self.iters[item.iter_index];
            self.current = Some(iter.value().clone());
            iter.next();
            self.push_if_valid(item.iter_index);
        }
    }
}

impl DbIterator for MergeIterator {
    fn valid(&self) -> bool {
        self.current.is_some()
    }

    fn key(&self) -> &str {
        &self.current.as_ref().expect("invalid merge iterator").key
    }

    fn value(&self) -> &Entry {
        self.current.as_ref().expect("invalid merge iterator")
    }

    fn next(&mut self) {
        self.advance();
    }
}

pub struct CompactionIterator {
    inner: MergeIterator,
    current: Option<Entry>,
    keep_tombstones: bool,
}

impl CompactionIterator {
    pub fn new(inner: MergeIterator) -> Self {
        let mut this = Self {
            inner,
            current: None,
            keep_tombstones: true,
        };
        this.advance();
        this
    }

    fn advance(&mut self) {
        self.current = None;

        while self.inner.valid() {
            let candidate = self.inner.value().clone();
            let user_key = candidate.key.clone();
            self.inner.next();

            while self.inner.valid() && self.inner.key() == user_key {
                self.inner.next();
            }

            if !self.keep_tombstones && matches!(candidate.value, crate::types::ValueRef::Tombstone) {
                continue;
            }

            self.current = Some(candidate);
            return;
        }
    }
}

impl DbIterator for CompactionIterator {
    fn valid(&self) -> bool {
        self.current.is_some()
    }

    fn key(&self) -> &str {
        &self.current.as_ref().expect("invalid compaction iterator").key
    }

    fn value(&self) -> &Entry {
        self.current.as_ref().expect("invalid compaction iterator")
    }

    fn next(&mut self) {
        self.advance();
    }
}
