//! Artifacts for the reader and limit tests, and the byte surgery that makes them hostile.
//!
//! The artifacts are written by the real writer; a hostile one is that artifact with a few bytes
//! replaced at an offset found from the directory, so repairing it is putting the bytes back.

use super::fixtures::{Input, config, eid};
use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::directory::{BLOCK_ENTRY_FIELDS, BlockEntry, ENTRY_LEN, parse_directory};
use pvec_container::spec::dir;
use pvec_container::writer::{ContainerWriter, EntityMeta};
use pvec_container::{Bounds, Header, Manifest};

/// An artifact and where the parts of it are.
#[derive(Clone)]
pub struct Art {
    pub bytes: Vec<u8>,
    pub header: Header,
    pub manifest: Manifest,
    pub entries: Vec<BlockEntry>,
}

impl Art {
    pub fn new(bytes: Vec<u8>) -> Art {
        let len = bytes.len() as u64;
        let header = Header::parse(&bytes[..bytes.len().min(4096)], len).expect("a valid header");
        let (m0, m1) =
            (header.manifest_offset as usize, (header.manifest_offset + header.manifest_length) as usize);
        let manifest = Manifest::parse(&bytes[m0..m1], &header, len).expect("a valid manifest");
        let d = manifest.directories[0];
        let entries = parse_directory(
            &bytes[d.offset as usize..(d.offset + d.length) as usize],
            u64::from(header.header_len),
            len,
        )
        .expect("a valid directory");
        Art { bytes, header, manifest, entries }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Where the directory starts.
    pub fn directory(&self) -> usize {
        self.manifest.directories[0].offset as usize
    }

    /// Absolute offset of a field of directory entry `i`.
    pub fn entry_field(&self, i: usize, name: &str) -> usize {
        let f = BLOCK_ENTRY_FIELDS.iter().find(|f| f.name == name).expect("a directory field");
        self.directory() + i * ENTRY_LEN + f.offset
    }

    /// Absolute offset of the directory pointer's own offset field in the manifest.
    pub fn pointer_offset_field(&self) -> usize {
        self.header.directory_table_offset as usize + dir::OFFSET
    }

    /// The index of the block of this kind and subtype.
    pub fn index_of(&self, kind: u16, subtype: u16) -> usize {
        self.entries
            .iter()
            .position(|e| e.kind == kind && e.subtype == subtype)
            .unwrap_or_else(|| panic!("no block {kind}/{subtype}"))
    }

    pub fn put_u32(&mut self, at: usize, v: u32) {
        self.bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn put_u64(&mut self, at: usize, v: u64) {
        self.bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    pub fn u32_at(&self, at: usize) -> u32 {
        u32::from_le_bytes([self.bytes[at], self.bytes[at + 1], self.bytes[at + 2], self.bytes[at + 3]])
    }

    pub fn with_u32(mut self, at: usize, v: u32) -> Art {
        self.put_u32(at, v);
        self
    }

    pub fn with_u64(mut self, at: usize, v: u64) -> Art {
        self.put_u64(at, v);
        self
    }
}

fn unit_meta(id: u32, cell: u32) -> EntityMeta {
    EntityMeta { id: eid(id), style: 0, cell, bounds: Bounds { min: [0.0, 0.0], max: [8.0, 8.0] } }
}

/// `n` lines in each of `cells` cells: one hot block per cell.
pub fn lines(n: u32, cells: u32) -> Art {
    let mut w = ContainerWriter::new(config(&[], None));
    let mut id = 0;
    for cell in 0..cells {
        for k in 0..n {
            let f = f64::from(k);
            w.push_line(unit_meta(id, cell), [f, 0.0], [f + 1.0, 2.0 + f]).expect("a line");
            id += 1;
        }
    }
    Art::new(w.finish().expect("the lines write"))
}

/// Splines of 10 control points and degree 3 (14 knots), so a ceiling of 8 control points or 12 knots
/// refuses them, and the two arenas hold more than one entry.
pub fn splines() -> Art {
    let mut w = ContainerWriter::new(config(&[], None));
    for id in 0..3u32 {
        let x: Vec<f64> = (0..10).map(|j| f64::from(j) + f64::from(id)).collect();
        let y: Vec<f64> = (0..10).map(|j| f64::from((j * 7) % 5)).collect();
        let mut knots = vec![0.0; 4];
        knots.extend((1..=6).map(f64::from));
        knots.extend([7.0; 4]);
        let spec = SplineSpec {
            degree: 3,
            knots: &knots,
            x: &x,
            y: &y,
            weights: None,
            closed: false,
            periodic: false,
        };
        w.push_spline(unit_meta(id, 0), spec).expect("a spline");
    }
    Art::new(w.finish().expect("the splines write"))
}

/// Composites nested `depth` deep (1 is a spline alone, 4 is NP-18's maximum): a chain
/// `10 <- 20 <- 21 <- ...` where each composite names the one before it.
pub fn composites(depth: u32) -> Art {
    let mut w = ContainerWriter::new(config(&[], None));
    let spec = SplineSpec {
        degree: 1,
        knots: &[0.0, 0.0, 1.0, 1.0],
        x: &[0.0, 2.0],
        y: &[0.0, 1.0],
        weights: None,
        closed: false,
        periodic: false,
    };
    w.push_spline(unit_meta(10, 0), spec).expect("a spline");
    let mut below = 10;
    for level in 2..=depth {
        let id = 20 + level;
        w.push_composite(unit_meta(id, 0), &[eid(below)], None, false).expect("a composite");
        below = id;
    }
    Art::new(w.finish().expect("the composites write"))
}

/// A mixed fixture input as an artifact.
pub fn from_input(input: &Input) -> Art {
    Art::new(input.write().expect("the fixture writes"))
}

fn align8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

/// `art` with `gaps[i]` zero bytes (a multiple of 8) before block `i`, and the directory moved to the
/// end of the longer file, with every offset a reader follows patched to match.
pub fn with_gaps(art: &Art, gaps: &[usize]) -> Art {
    assert_eq!(gaps.len(), art.entries.len());
    let dir_len = art.manifest.directories[0].length as usize;
    let mut out = art.bytes[..art.entries[0].offset as usize].to_vec();
    let mut new_offsets = Vec::new();
    for (i, e) in art.entries.iter().enumerate() {
        assert_eq!(gaps[i] % 8, 0);
        out.resize(out.len() + gaps[i], 0);
        new_offsets.push(out.len());
        let (a, b) = (e.offset as usize, e.offset as usize + e.stored_length as usize);
        out.extend_from_slice(&art.bytes[a..b]);
    }
    out.resize(align8(out.len()), 0);
    let new_dir = out.len();
    out.extend_from_slice(&art.bytes[art.directory()..art.directory() + dir_len]);
    let mut moved = Art { bytes: out, ..art.clone() };
    moved.put_u64(art.pointer_offset_field(), new_dir as u64);
    for (i, off) in new_offsets.iter().enumerate() {
        let at =
            new_dir + i * ENTRY_LEN + BLOCK_ENTRY_FIELDS.iter().find(|f| f.name == "offset").unwrap().offset;
        moved.put_u64(at, *off as u64);
    }
    Art::new(moved.bytes)
}
