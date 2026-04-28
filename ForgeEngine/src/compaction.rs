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

/// `TableIterator` is a structure that serves as a wrapper around the `SstableIterator`.
/// It implements functionality to iterate over a specific table structure in a database or storage system.
///
/// # Attributes
///
/// * `inner` - An instance of `SstableIterator` that handles the actual iteration logic.
///
/// # Example
///
/// ```rust
/// use ForgeEngine::TableIterator;
///
/// let sstable_iterator = SstableIterator::new(...); // Initialize the SstableIterator
/// let table_iterator = TableIterator { inner: sstable_iterator };
///
/// println!("{:?}", table_iterator);
/// ```
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

/// A struct representing an item in a heap, used in merging iterators
/// where elements need to be compared and ordered based on specific fields.
///
/// # Fields
///
/// * `key` - A `String` field that serves as the primary identifier or sort key for this item.
/// * `seq` - A `u64` field used for additional ordering. Typically represents a sequence number or timestamp.
/// * `iter_index` - A `usize` field that indicates the originating iterator's index if this struct is used in
///   an iterator merging scenario. Useful for tracing which iterator produced this item.
///
/// # Traits
///
/// * `Debug` - Enables formatting of the struct using the `{:?}` formatter for debugging purposes.
/// * `Clone` - Allows the struct to be cloned, creating an identical copy.
/// * `Eq` - Enables equality comparison.
/// * `PartialEq` - Allows partial equality comparison, enabling the use of the `==` operator.
///
/// # Example
///
/// ```
/// let heap_item = HeapItem {
///     key: String::from("example"),
///     seq: 42,
///     iter_index: 0,
/// };
///
/// println!("{:?}", heap_item); // Output: HeapItem { key: "example", seq: 42, iter_index: 0 }
/// ```
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

/// A `MergeIterator` is an iterator that merges multiple database iterators into a single
/// sorted iterator. It is useful when traversing multiple sources of sorted data
/// while maintaining a consistent ordering.
///
/// # Fields
///
/// * `iters` - A vector of boxed database iterators (`DbIterator`), each of which
///   provides a sorted sequence of entries. These iterators serve as the input
///   sources for the `MergeIterator`.
///
/// * `heap` - A min-heap implemented as a `BinaryHeap` that is used to efficiently
///   keep track of the next smallest element among the entries provided by the database
///   iterators. The heap ensures that the elements are processed in sorted order.
///
/// * `current` - An optional field that holds the currently active entry (`Entry`)
///   being yielded by the iterator. If the iterator is exhausted, this will be `None`.
///
/// # Features
///
/// - Merges and yields entries from multiple sorted iterators in a globally sorted order.
/// - Uses a heap-based approach to maintain efficient merging.
/// - Supports any iterable source that implements the `DbIterator` trait.
///
/// # Notes
///
/// The `DbIterator` trait is expected to define the necessary operations for traversing
/// entries in a sorted order, such as `next()` for advancing the iterator.
///
/// # Example
///
/// ```rust
/// use ForgeEngine::{MergeIterator, DbIterator, Entry};
///
/// // Assume `DbIterator` and `Entry` are properly implemented and instantiated:
/// let iter1: Box<dyn DbIterator> = /* ... */;
/// let iter2: Box<dyn DbIterator> = /* ... */;
/// let iter3: Box<dyn DbIterator> = /* ... */;
///
/// let mut merge_iter = MergeIterator {
///     iters: vec![iter1, iter2, iter3],
///     heap: BinaryHeap::new(),
///     current: None,
/// };
///
/// while let Some(entry) = merge_iter.next() {
///     println!("{:?}", entry);
/// }
/// ```
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

/// The `CompactionIterator` is a utility structure used during the compaction process
/// the storage engine. It iterates over a sequence of entries, filters
/// out unneeded entries, and prepares the data for compaction.
///
/// # Fields
///
/// * `inner` - A `MergeIterator` that serves as the underlying iterator.
///   It provides the raw sequence of entries to be processed by the `CompactionIterator`.
///
/// * `current` - An `Option<Entry>` that holds the current entry being processed
///   by the iterator. If `None`, the iterator has reached the end of the sequence.
///
/// * `keep_tombstones` - A boolean flag indicating whether tombstone entries
///   (markers for deleted data) should be retained during the compaction process.
///   If `true`, tombstone entries are preserved; if `false`, they are skipped.
///
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

            if !self.keep_tombstones && matches!(candidate.value, crate::types::ValueRef::Tombstone)
            {
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
        &self
            .current
            .as_ref()
            .expect("invalid compaction iterator")
            .key
    }

    fn value(&self) -> &Entry {
        self.current.as_ref().expect("invalid compaction iterator")
    }

    fn next(&mut self) {
        self.advance();
    }
}
