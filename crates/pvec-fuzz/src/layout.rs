//! Where the parts of a container are, for the code that has to find them without trusting them: the
//! mutator, the reseal and the seed extraction.

use pvec_container::directory::{BlockEntry, parse_directory};
use pvec_container::manifest::DirectoryPointer;
use pvec_container::{Header, Manifest};

/// A container prefix that parsed.
pub struct Layout {
    /// The header.
    pub header: Header,
    /// The manifest.
    pub manifest: Manifest,
    /// The block directory.
    pub entries: Vec<BlockEntry>,
    /// Where the directory is.
    pub directory: (usize, usize),
}

fn slice(bytes: &[u8], offset: u64, length: u64) -> Option<&[u8]> {
    let end = offset.checked_add(length)?;
    bytes.get(usize::try_from(offset).ok()?..usize::try_from(end).ok()?)
}

impl Layout {
    /// The header, manifest and directory of `bytes`, or `None` when any of them does not parse.
    pub fn of(bytes: &[u8]) -> Option<Layout> {
        let len = bytes.len() as u64;
        let header = Header::parse(&bytes[..bytes.len().min(4096)], len).ok()?;
        let manifest =
            Manifest::parse(slice(bytes, header.manifest_offset, header.manifest_length)?, &header, len)
                .ok()?;
        let pointer = manifest.directories.iter().find(|d| d.kind == DirectoryPointer::KIND_BLOCK)?;
        let table = slice(bytes, pointer.offset, pointer.length)?;
        let entries = parse_directory(table, u64::from(header.header_len), len).ok()?;
        let directory = (usize::try_from(pointer.offset).ok()?, usize::try_from(pointer.length).ok()?);
        Some(Layout { header, manifest, entries, directory })
    }

    /// The stored bytes of block `i`, when they lie inside `bytes`.
    pub fn stored<'a>(&self, bytes: &'a [u8], i: usize) -> Option<&'a [u8]> {
        let e = self.entries.get(i)?;
        slice(bytes, e.offset, u64::from(e.stored_length))
    }

    /// The manifest as a byte range of the file.
    pub fn manifest_range(&self) -> (usize, usize) {
        (self.header.manifest_offset as usize, self.header.manifest_length as usize)
    }
}
