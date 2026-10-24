//! The reference reader: what a `.pvec` reader has to agree with.
//!
//! P4.5's safe reader does not exist yet. Until it does, this is the stand-in, built from the parsers
//! that do: the header, the manifest, the directory, the hot blocks, the advanced blocks and the
//! component frame. It is strict on purpose. It reads the canonical layout the P4.4 writer produces
//! and refuses anything else, so a mutated artifact has to come back as an error here, and a reader
//! that accepts it is the one in the wrong.
//!
//! Every slice is taken with a checked range, so a lying length is an error and never a panic.

use pvec_capabilities::{Reader, Registry, Version};
use pvec_components::read_streams;
use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::advanced::{AdvancedView, BlockSlices, CurveView};
use pvec_container::blocks::hot::{HotBlockView, HotKind};
use pvec_container::directory::{BlockEntry, BlockKind, parse_directory_checked};
use pvec_container::{ByteRange, DirectoryPointer, Header, Manifest};
use std::collections::BTreeSet;
use std::fmt;

/// Where in the read a refusal happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// The 64 byte header.
    Header,
    /// The manifest.
    Manifest,
    /// The block directory.
    Directory,
    /// The layout of the file: canonical order, padding, what the directory ends at.
    Layout,
    /// A hot block.
    HotBlock,
    /// The advanced curve blocks.
    Records,
    /// The component stream set.
    Components,
    /// What the blocks and the directory say about each other.
    Consistency,
}

/// A refusal: the stage and a message that names what was wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    /// Where it happened.
    pub stage: Stage,
    /// What was wrong, naming the rule or the limit where there is one.
    pub detail: String,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.stage, self.detail)
    }
}

impl std::error::Error for ReadError {}

fn refuse<T>(stage: Stage, detail: impl Into<String>) -> Result<T, ReadError> {
    Err(ReadError { stage, detail: detail.into() })
}

/// One entity as the file holds it, with the numbers exactly as stored.
#[derive(Clone, Debug, PartialEq)]
pub enum ReadEntity {
    /// A line segment.
    Line {
        /// Entity id.
        id: u32,
        /// Style reference.
        style: u32,
        /// Start point.
        a: [f64; 2],
        /// End point.
        b: [f64; 2],
    },
    /// A polyline.
    Polyline {
        /// Entity id.
        id: u32,
        /// Style reference.
        style: u32,
        /// X of each vertex.
        x: Vec<f64>,
        /// Y of each vertex.
        y: Vec<f64>,
    },
    /// A circle or a circular arc.
    CircleArc {
        /// Entity id.
        id: u32,
        /// Style reference.
        style: u32,
        /// Centre.
        centre: [f64; 2],
        /// Radius.
        radius: f64,
        /// Start angle and signed sweep.
        start_sweep: [f64; 2],
    },
    /// An ellipse or an elliptical arc.
    EllipseArc {
        /// Entity id.
        id: u32,
        /// Style reference.
        style: u32,
        /// Centre.
        centre: [f64; 2],
        /// The two semi-axes.
        radii: [f64; 2],
        /// Rotation, start angle and signed sweep.
        angles: [f64; 3],
    },
    /// A B-spline, rational when `weights` is there.
    Spline {
        /// Entity id.
        id: u32,
        /// Style reference.
        style: u32,
        /// Degree.
        degree: usize,
        /// The closed flag.
        closed: bool,
        /// Knots, with multiplicities.
        knots: Vec<f64>,
        /// X of each control point.
        x: Vec<f64>,
        /// Y of each control point.
        y: Vec<f64>,
        /// Weights.
        weights: Option<Vec<f64>>,
    },
    /// A composite curve, by record index.
    Composite {
        /// Entity id.
        id: u32,
        /// Style reference.
        style: u32,
        /// The closed flag.
        closed: bool,
        /// Record indices of the segments.
        segments: Vec<u32>,
        /// `(t1, t2)` per segment, flattened.
        trims: Option<Vec<f64>>,
    },
}

impl ReadEntity {
    /// The entity id.
    pub fn id(&self) -> u32 {
        match self {
            ReadEntity::Line { id, .. }
            | ReadEntity::Polyline { id, .. }
            | ReadEntity::CircleArc { id, .. }
            | ReadEntity::EllipseArc { id, .. }
            | ReadEntity::Spline { id, .. }
            | ReadEntity::Composite { id, .. } => *id,
        }
    }

    /// Every stored real as its bit pattern, in storage order, so two entities can be compared
    /// without `-0.0 == 0.0` or a NaN getting in the way.
    pub fn bits(&self) -> Vec<u64> {
        let f = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<u64>>();
        match self {
            ReadEntity::Line { a, b, .. } => [f(a), f(b)].concat(),
            ReadEntity::Polyline { x, y, .. } => [f(x), f(y)].concat(),
            ReadEntity::CircleArc { centre, radius, start_sweep, .. } => {
                [f(centre), f(&[*radius]), f(start_sweep)].concat()
            }
            ReadEntity::EllipseArc { centre, radii, angles, .. } => {
                [f(centre), f(radii), f(angles)].concat()
            }
            ReadEntity::Spline { knots, x, y, weights, .. } => {
                [f(knots), f(x), f(y), weights.as_deref().map(f).unwrap_or_default()].concat()
            }
            ReadEntity::Composite { trims, .. } => trims.as_deref().map(f).unwrap_or_default(),
        }
    }
}

/// What a successful read gives back.
#[derive(Clone, Debug, PartialEq)]
pub struct Opened {
    /// The header.
    pub header: Header,
    /// The manifest.
    pub manifest: Manifest,
    /// The directory entries, in file order.
    pub entries: Vec<BlockEntry>,
    /// Every entity, ascending by id.
    pub entities: Vec<ReadEntity>,
    /// The capability ids of the component streams, ascending.
    pub stream_capabilities: Vec<u32>,
}

fn slice(bytes: &[u8], start: u64, len: u64) -> Option<&[u8]> {
    let end = start.checked_add(len)?;
    bytes.get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)
}

/// Reads a whole artifact, or says what is wrong with it.
pub fn read(bytes: &[u8]) -> Result<Opened, ReadError> {
    let len = bytes.len() as u64;
    let prefix = &bytes[..bytes.len().min(4096)];
    let header = Header::parse(prefix, len).map_err(|e| ReadError { stage: Stage::Header, detail: e.to_string() })?;
    if header.manifest_offset != u64::from(header.header_len) {
        return refuse(
            Stage::Layout,
            format!("the manifest starts at {}, not straight after the header", header.manifest_offset),
        );
    }
    let manifest_bytes = slice(bytes, header.manifest_offset, header.manifest_length)
        .ok_or_else(|| ReadError { stage: Stage::Manifest, detail: "the manifest lies outside the file".into() })?;
    let manifest = Manifest::parse(manifest_bytes, &header, len)
        .map_err(|e| ReadError { stage: Stage::Manifest, detail: e.to_string() })?;
    if manifest.directories.len() != 1 || manifest.directories[0].kind != DirectoryPointer::KIND_BLOCK {
        return refuse(
            Stage::Manifest,
            format!("expected one block directory pointer, got {:?}", manifest.directories),
        );
    }
    if manifest.capabilities.windows(2).any(|w| w[0].id >= w[1].id) {
        return refuse(Stage::Layout, "capabilities are not strictly ascending by id");
    }
    let ptr = manifest.directories[0];
    if ptr.offset.checked_add(ptr.length) != Some(len) {
        return refuse(
            Stage::Layout,
            format!("the directory ends at {}, the file at {len}", ptr.offset.saturating_add(ptr.length)),
        );
    }
    let table = slice(bytes, ptr.offset, ptr.length)
        .ok_or_else(|| ReadError { stage: Stage::Directory, detail: "the directory lies outside the file".into() })?;
    let manifest_range = ByteRange { offset: header.manifest_offset, length: header.manifest_length };
    let directory_range = ByteRange { offset: ptr.offset, length: ptr.length };
    let entries =
        parse_directory_checked(table, u64::from(header.header_len), len, manifest_range, directory_range)
            .map_err(|e| ReadError { stage: Stage::Directory, detail: e.to_string() })?;
    if entries.is_empty() {
        return refuse(Stage::Directory, "no blocks");
    }

    let keys: Vec<(u16, u32, u16)> = entries.iter().map(|e| (e.kind, e.cell, e.subtype)).collect();
    if keys.windows(2).any(|w| w[0] >= w[1]) {
        return refuse(
            Stage::Layout,
            format!("blocks are not in strictly ascending (kind, cell, geometry type) order: {keys:?}"),
        );
    }

    // Every byte outside a block and the directory is zero, and no two blocks share a byte.
    let mut at = manifest_range.offset + manifest_range.length;
    for e in &entries {
        let (start, end) = (e.offset, e.offset + u64::from(e.stored_length));
        if start < at {
            return refuse(Stage::Layout, format!("the block at {start} overlaps what comes before it"));
        }
        let gap = slice(bytes, at, start - at)
            .ok_or_else(|| ReadError { stage: Stage::Layout, detail: "a block lies outside the file".into() })?;
        if gap.iter().any(|b| *b != 0) {
            return refuse(Stage::Layout, format!("non-zero padding before the block at {start}"));
        }
        at = end;
    }
    let tail = slice(bytes, at, ptr.offset.saturating_sub(at))
        .ok_or_else(|| ReadError { stage: Stage::Layout, detail: "the blocks run past the directory".into() })?;
    if at > ptr.offset || tail.iter().any(|b| *b != 0) {
        return refuse(Stage::Layout, "non-zero padding before the directory");
    }

    let mut entities: Vec<ReadEntity> = Vec::new();
    let mut ids: BTreeSet<u32> = BTreeSet::new();
    let mut advanced: Vec<(u16, AlignedBytes)> = Vec::new();
    let mut record_count = 0usize;
    let mut stream_ids: Vec<u32> = Vec::new();
    for e in &entries {
        let raw = slice(bytes, e.offset, u64::from(e.stored_length))
            .ok_or_else(|| ReadError { stage: Stage::Layout, detail: "a block lies outside the file".into() })?;
        let aligned = AlignedBytes::from_bytes(raw);
        match e.block_kind() {
            Some(BlockKind::Hot) => {
                let v = HotBlockView::parse(aligned.as_bytes())
                    .map_err(|x| ReadError { stage: Stage::HotBlock, detail: x.to_string() })?;
                if v.kind().id() != e.subtype || v.len() != e.count as usize {
                    return refuse(Stage::Consistency, "the directory entry disagrees with the hot block");
                }
                for i in 0..v.len() {
                    entities.push(hot_entity(&v, i)?);
                }
            }
            Some(BlockKind::Records) | Some(BlockKind::Arena) => {
                if e.block_kind() == Some(BlockKind::Records) {
                    record_count = e.count as usize;
                }
                advanced.push(((e.kind << 8) | e.subtype, aligned));
            }
            Some(BlockKind::Components) => {
                let reader = Reader { version: Version::new(1, 0, 0), implemented: Vec::new() };
                let opened = read_streams(aligned.as_bytes(), &Registry::v1(), &reader)
                    .map_err(|x| ReadError { stage: Stage::Components, detail: format!("{x:?}") })?;
                stream_ids = opened.skipped.iter().map(|s| s.capability.get()).collect();
                stream_ids.sort_unstable();
            }
            None => return refuse(Stage::Directory, format!("unknown block kind {}", e.kind)),
        }
    }
    if !advanced.is_empty() {
        let get = |k: u16, s: u16| {
            advanced
                .iter()
                .find(|(key, _)| *key == ((k << 8) | s))
                .map(|(_, b)| b.as_bytes())
                .ok_or(ReadError { stage: Stage::Records, detail: format!("no block {k}/{s}") })
        };
        let slices = BlockSlices {
            records: get(2, 0)?,
            knots: get(3, 1)?,
            weights: get(3, 2)?,
            control_points: get(3, 3)?,
            segments: get(3, 4)?,
            trims: get(3, 5)?,
        };
        let view = AdvancedView::parse(slices)
            .map_err(|x| ReadError { stage: Stage::Records, detail: x.to_string() })?;
        view.check_nesting().map_err(|x| ReadError { stage: Stage::Records, detail: format!("nesting: {x}") })?;
        if view.len() != record_count {
            return refuse(Stage::Consistency, "record count differs from the directory");
        }
        for i in 0..view.len() {
            let rec = view
                .record(i)
                .ok_or_else(|| ReadError { stage: Stage::Records, detail: "record missing".into() })?;
            let curve =
                view.curve(i).map_err(|x| ReadError { stage: Stage::Records, detail: format!("curve {i}: {x}") })?;
            entities.push(match curve {
                CurveView::Spline(s) => ReadEntity::Spline {
                    id: rec.id,
                    style: rec.style,
                    degree: s.degree,
                    closed: s.closed,
                    knots: s.knots.to_vec(),
                    x: s.x.to_vec(),
                    y: s.y.to_vec(),
                    weights: s.weights.map(<[f64]>::to_vec),
                },
                CurveView::Composite(c) => ReadEntity::Composite {
                    id: rec.id,
                    style: rec.style,
                    closed: c.closed,
                    segments: c.segments.to_vec(),
                    trims: c.trims.map(<[f64]>::to_vec),
                },
            });
        }
    }
    for e in &entities {
        if !ids.insert(e.id()) {
            return refuse(Stage::Consistency, format!("entity {} is in two blocks", e.id()));
        }
    }
    entities.sort_by_key(ReadEntity::id);
    Ok(Opened { header, manifest, entries, entities, stream_capabilities: stream_ids })
}

fn hot_entity(v: &HotBlockView<'_>, i: usize) -> Result<ReadEntity, ReadError> {
    let missing = || ReadError { stage: Stage::HotBlock, detail: "a column is missing".into() };
    let (id, style) = (v.entity_ids()[i], v.styles()[i]);
    let col = |name: &str| v.column(name).and_then(|c| c.get(i)).copied().ok_or_else(missing);
    Ok(match v.kind() {
        HotKind::Line => ReadEntity::Line {
            id,
            style,
            a: [col("x0")?, col("y0")?],
            b: [col("x1")?, col("y1")?],
        },
        HotKind::Polyline => {
            let (x, y) = v.polyline(i).ok_or_else(missing)?;
            ReadEntity::Polyline { id, style, x: x.to_vec(), y: y.to_vec() }
        }
        HotKind::CircleArc => ReadEntity::CircleArc {
            id,
            style,
            centre: [col("cx")?, col("cy")?],
            radius: col("radius")?,
            start_sweep: [col("start")?, col("sweep")?],
        },
        HotKind::EllipseArc => ReadEntity::EllipseArc {
            id,
            style,
            centre: [col("cx")?, col("cy")?],
            radii: [col("rx")?, col("ry")?],
            angles: [col("rotation")?, col("start")?, col("sweep")?],
        },
    })
}
