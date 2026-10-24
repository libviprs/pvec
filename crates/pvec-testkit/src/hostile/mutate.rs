//! Byte-level mutations and where things are in a valid artifact.

use crate::reference::read;

/// One edit of a byte string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Overwrite the bytes at `at`.
    Put {
        /// Where.
        at: usize,
        /// The new bytes.
        bytes: Vec<u8>,
    },
    /// Cut the file to `len` bytes.
    Truncate {
        /// The new length.
        len: usize,
    },
    /// Append bytes.
    Append {
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// Remove `len` bytes at `at`, closing the gap.
    Remove {
        /// Where.
        at: usize,
        /// How many.
        len: usize,
    },
}

impl Op {
    /// Overwrites a little-endian `u16`.
    pub fn u16(at: usize, v: u16) -> Op {
        Op::Put { at, bytes: v.to_le_bytes().to_vec() }
    }
    /// Overwrites a little-endian `u32`.
    pub fn u32(at: usize, v: u32) -> Op {
        Op::Put { at, bytes: v.to_le_bytes().to_vec() }
    }
    /// Overwrites a little-endian `u64`.
    pub fn u64(at: usize, v: u64) -> Op {
        Op::Put { at, bytes: v.to_le_bytes().to_vec() }
    }
    /// Overwrites an `f64` with these bits.
    pub fn f64(at: usize, v: f64) -> Op {
        Op::Put { at, bytes: v.to_bits().to_le_bytes().to_vec() }
    }
    /// Overwrites one byte.
    pub fn u8(at: usize, v: u8) -> Op {
        Op::Put { at, bytes: vec![v] }
    }
}

/// `base` with `ops` applied in order. A `Put` outside the file is a bug in the case, so it panics.
pub fn apply(base: &[u8], ops: &[Op]) -> Vec<u8> {
    let mut v = base.to_vec();
    for op in ops {
        match op {
            Op::Put { at, bytes } => {
                assert!(
                    at + bytes.len() <= v.len(),
                    "a Put at {at} of {} bytes is outside the file",
                    bytes.len()
                );
                v[*at..at + bytes.len()].copy_from_slice(bytes);
            }
            Op::Truncate { len } => v.truncate(*len),
            Op::Append { bytes } => v.extend_from_slice(bytes),
            Op::Remove { at, len } => {
                v.drain(*at..at + len);
            }
        }
    }
    v
}

/// Where one block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockLoc {
    /// The block kind id.
    pub kind: u16,
    /// The subtype: geometry type for a hot block, payload kind for an arena, 0 otherwise.
    pub subtype: u16,
    /// The spatial cell.
    pub cell: u32,
    /// Absolute offset of the block.
    pub offset: usize,
    /// Stored length.
    pub len: usize,
    /// Absolute offset of the block's directory entry.
    pub entry: usize,
}

/// Where the parts of a valid artifact are, read with the reference reader so that a base the reader
/// cannot open is caught when the catalog is built and not when a case runs.
#[derive(Clone, Debug)]
pub struct Layout {
    /// The length of the file.
    pub len: usize,
    /// Absolute offset of the manifest.
    pub manifest: usize,
    /// Length of the manifest.
    pub manifest_len: usize,
    /// Where the directory-pointer table is (inside the manifest).
    pub pointer_table: usize,
    /// Absolute offset of the block directory.
    pub directory: usize,
    /// Length of the block directory.
    pub directory_len: usize,
    /// Every block, in directory order.
    pub blocks: Vec<BlockLoc>,
}

impl Layout {
    /// The layout of a valid artifact. Panics if the reference reader refuses it.
    pub fn of(bytes: &[u8]) -> Layout {
        let opened = read(bytes).unwrap_or_else(|e| panic!("a base has to be a valid artifact: {e}"));
        let h = &opened.header;
        let directory = opened.manifest.directories[0];
        let blocks = opened
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| BlockLoc {
                kind: e.kind,
                subtype: e.subtype,
                cell: e.cell,
                offset: e.offset as usize,
                len: e.stored_length as usize,
                entry: directory.offset as usize + i * pvec_container::directory::ENTRY_LEN,
            })
            .collect();
        Layout {
            len: bytes.len(),
            manifest: h.manifest_offset as usize,
            manifest_len: h.manifest_length as usize,
            pointer_table: h.directory_table_offset as usize,
            directory: directory.offset as usize,
            directory_len: directory.length as usize,
            blocks,
        }
    }

    /// The first block of this kind and subtype.
    pub fn block(&self, kind: u16, subtype: u16) -> Option<&BlockLoc> {
        self.blocks.iter().find(|b| b.kind == kind && b.subtype == subtype)
    }
}

/// The offset of a field in a directory entry, from the container's own field table.
pub fn entry_field(name: &str) -> usize {
    pvec_container::directory::BLOCK_ENTRY_FIELDS
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no directory entry field `{name}`"))
        .offset
}

/// The offset of a field in a table of `(name, offset)` rows such as the header and arena tables.
pub fn field_at(table: &[pvec_container::spec::FieldSpec], name: &str) -> usize {
    table.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no field `{name}`")).offset
}
