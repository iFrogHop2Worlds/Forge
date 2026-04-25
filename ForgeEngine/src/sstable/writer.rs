use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::Path;

use crate::sstable::block::write_entry;
use crate::sstable::index::SparseIndex;
use crate::types::{Entry, Result};

pub fn write_sstable(path: &Path, index_path: &Path, entries: &[Entry], index_stride: usize) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    let mut sparse = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let offset = writer.stream_position()?;
        if i % index_stride.max(1) == 0 {
            sparse.push((entry.key.clone(), offset));
        }
        write_entry(&mut writer, entry)?;
    }

    writer.flush()?;
    SparseIndex::new(sparse).save(index_path)?;
    Ok(())
}
