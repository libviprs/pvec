use crate::components::{Skipped, Stream};
use crate::{Curve, EntityId, Manifest, PvecError};
use core::f64::consts::TAU;
use pvec_capabilities::{FORMAT_VERSION, Reader, Registry};
use pvec_components::read_streams;
use pvec_container::blocks::advanced::CurveView;
use pvec_container::blocks::hot::HotKind;
use pvec_container::directory::{BlockKind, STRADDLER_CELL_FIRST};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::{BlockSource, Bytes, MemorySource, SourceError};
use pvec_geometry::{
    BSpline, Circle, Composite, Curve2d, Ellipse, GeometryError, Line, Placement2, Point2, Polyline,
    RationalBSpline, Trim, Trimmed, Vector2,
};
use pvec_numeric::detmath;

/// Where the reader gets its bytes: the file in memory, or (feature `http`) a callback that returns byte
/// ranges. The reader is written against `BlockSource`, and a facade type cannot name the container's, so
/// this is the one type that implements it for both (contract).
pub(crate) enum Source {
    Memory(MemorySource<Vec<u8>>),
    #[cfg(feature = "http")]
    Callback {
        len: u64,
        read: Box<dyn Fn(u64, u32) -> Result<Vec<u8>, String> + Send + Sync>,
    },
}

impl BlockSource for Source {
    fn len(&self) -> u64 {
        match self {
            Source::Memory(m) => BlockSource::len(m),
            #[cfg(feature = "http")]
            Source::Callback { len, .. } => *len,
        }
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        match self {
            Source::Memory(m) => m.read_range(offset, len),
            #[cfg(feature = "http")]
            Source::Callback { len: source_len, read } => {
                match offset.checked_add(u64::from(len)) {
                    Some(end) if end <= *source_len => {}
                    _ => {
                        return Err(SourceError::OutOfRange {
                            offset,
                            len: u64::from(len),
                            source_len: *source_len,
                        });
                    }
                }
                read(offset, len).map(Bytes::new).map_err(SourceError::Io)
            }
        }
    }
}

/// An entity read from a file: the leaf of the hot block it came from (`None` for an advanced curve), its id,
/// its curve and its style reference.
pub type LoadedEntity = (Option<u32>, EntityId, Curve, u32);

/// Reads a PVec file held in memory, or (feature `http`) behind a range callback.
pub struct PvecReader {
    pub(crate) inner: ContainerReader<Source>,
    manifest: Manifest,
}

pub(crate) fn container(e: pvec_container::reader::ReadError) -> PvecError {
    PvecError::Read { rule: e.rule() }
}

fn geometry(e: GeometryError) -> PvecError {
    PvecError::Read { rule: e.rule().to_string() }
}

/// An arc of a circle or an ellipse from the stored start angle and signed sweep. A sweep of one full
/// turn from angle zero is the whole carrier. A negative sweep is a reversed trim of the interval it
/// covers, and the interval's start is brought into `[0, tau)`, which is the form the writer stores
/// it from.
fn arc(basis: Curve, start: f64, sweep: f64) -> Result<Curve, PvecError> {
    if start == 0.0 && sweep == TAU {
        return Ok(basis);
    }
    let low = if sweep < 0.0 { start + sweep } else { start };
    let low = if (0.0..TAU).contains(&low) { low } else { low.rem_euclid(TAU) };
    let low = if low >= TAU { 0.0 } else { low };
    let trim = Trim { start: low, span: sweep.abs(), reversed: sweep < 0.0 };
    Trimmed::from_normalised(basis, trim).map(Curve::Trimmed).map_err(geometry)
}

/// The unit x axis of an ellipse frame whose stored rotation is `rot`. The writer stores the angle of
/// the axis by the deterministic arctangent, so the axis returned here is one whose angle by that same
/// arctangent is exactly `rot`: it is looked for among the neighbours of the libm direction, nearest
/// first, and the file is refused if none is found. That makes the next write store `rot` again.
fn axis(rot: f64) -> Result<Vector2, PvecError> {
    if rot == 0.0 {
        return Ok(Vector2::new(1.0, 0.0));
    }
    let (s, c) = rot.sin_cos();
    let mut steps: Vec<(i64, i64)> = (-8..=8).flat_map(|dx| (-8..=8).map(move |dy| (dx, dy))).collect();
    steps.sort_by_key(|(dx, dy)| dx.abs() + dy.abs());
    let nudge = |v: f64, by: i64| f64::from_bits((v.to_bits() as i64 + by) as u64);
    for (dx, dy) in steps {
        let (x, y) = (nudge(c, dx), nudge(s, dy));
        if detmath::atan2(y, x) == rot
            && Placement2::from_stored(Point2::new(0.0, 0.0), Vector2::new(x, y)).is_ok()
        {
            return Ok(Vector2::new(x, y));
        }
    }
    Err(PvecError::Read { rule: "HOT-ellipse-rotation: no unit axis has this stored angle".to_string() })
}

/// A stored segment as a trimmed line. A line is an origin, a unit direction and a length, so the far
/// end is `origin + length * direction` rounded, and the obvious values land one ulp off `b` for most
/// diagonals. The writer stores the far end it evaluates, so a rewrite would move it. The direction
/// and the length are therefore looked for among the neighbours of the obvious ones, nearest first,
/// until the far end evaluates to `b` exactly. If none does, the obvious values are used.
fn segment(a: Point2, b: Point2) -> Result<Curve, PvecError> {
    let line = Line::new(a, a.vector_to(b)).map_err(geometry)?;
    let (d, len) = (line.direction(), a.distance(b));
    let nudge = |v: f64, by: i64| f64::from_bits((v.to_bits() as i64 + by) as u64);
    let build = |dx, dy, ds| -> Option<Curve> {
        let direction = Vector2::new(nudge(d.x, dx), nudge(d.y, dy));
        let basis = Curve::Line(Line::from_stored(a, direction).ok()?);
        let trim = Trim { start: 0.0, span: nudge(len, ds), reversed: false };
        Trimmed::from_normalised(basis, trim).ok().map(Curve::Trimmed)
    };
    let exact = |c: &Curve| c.eval(c.domain().hi()).is_ok_and(|p| p == b);
    // The obvious values are right for most segments, so the neighbours are only listed for the rest.
    if let Some(c) = build(0, 0, 0).filter(exact) {
        return Ok(c);
    }
    let mut steps: Vec<(i64, i64, i64)> = Vec::new();
    for dx in -4..=4 {
        for dy in -4..=4 {
            for ds in -4..=4 {
                steps.push((dx, dy, ds));
            }
        }
    }
    steps.sort_by_key(|(dx, dy, ds)| dx.abs() + dy.abs() + ds.abs());
    for (dx, dy, ds) in steps {
        if let Some(c) = build(dx, dy, ds).filter(exact) {
            return Ok(c);
        }
    }
    build(0, 0, 0).ok_or(PvecError::Read { rule: "NP-12".to_string() })
}

/// An advanced curve as a semantic one. A composite names other records of its chunk by index.
fn advanced(
    view: &pvec_container::reader::CheckedView<'_>,
    index: usize,
    depth: usize,
) -> Result<Curve, PvecError> {
    if depth > 4 {
        return Err(PvecError::Read { rule: "NP-18".to_string() });
    }
    match view.curve(index).map_err(container)? {
        CurveView::Spline(s) => {
            let points: Vec<Point2> = s.x.iter().zip(s.y).map(|(x, y)| Point2::new(*x, *y)).collect();
            match s.weights {
                Some(w) => {
                    let c = RationalBSpline::new(s.degree, points, s.knots.to_vec(), w.to_vec())
                        .map_err(geometry)?;
                    Ok(Curve::RationalBSpline(if s.closed {
                        c.declare_closed().map_err(geometry)?
                    } else {
                        c
                    }))
                }
                None => {
                    let c = BSpline::new(s.degree, points, s.knots.to_vec()).map_err(geometry)?;
                    Ok(Curve::BSpline(if s.closed { c.declare_closed().map_err(geometry)? } else { c }))
                }
            }
        }
        CurveView::Composite(c) => {
            if c.trims.is_some() {
                return Err(PvecError::Read {
                    rule: "a composite with trimmed segments is not read yet".to_string(),
                });
            }
            let parts = c
                .segments
                .iter()
                .map(|i| advanced(view, *i as usize, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            let whole = Composite::new(parts).map_err(geometry)?;
            Ok(Curve::Composite(if c.closed { whole.declare_closed().map_err(geometry)? } else { whole }))
        }
    }
}

impl PvecReader {
    /// Opens a file, checking the header, manifest and directory.
    pub fn open(bytes: Vec<u8>) -> Result<PvecReader, PvecError> {
        PvecReader::from_source(Source::Memory(MemorySource::new(bytes)), ReaderConfig::default())
    }

    pub(crate) fn from_source(source: Source, config: ReaderConfig) -> Result<PvecReader, PvecError> {
        let inner = ContainerReader::open(source, config).map_err(container)?;
        let manifest = Manifest::from_container(inner.manifest());
        Ok(PvecReader { inner, manifest })
    }

    /// Opens a file of `len` bytes that is read through `read(offset, len)`, which returns exactly `len`
    /// bytes starting at `offset` (an HTTP range request, a cache, a file). Opening asks for the first
    /// bytes of the file, then the manifest and the block directory when they did not fit, and reads
    /// nothing else: a block is read when a call needs it, one range per block, and the reader keeps what
    /// it decoded. What `read` returns is checked like any other bytes: a reply of another length, or an
    /// error, is a [`PvecError`], and a range outside the file is refused before `read` is called.
    ///
    /// Behind the `http` feature, which adds this call and no type.
    #[cfg(feature = "http")]
    pub fn open_ranges(
        len: u64,
        read: impl Fn(u64, u32) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    ) -> Result<PvecReader, PvecError> {
        PvecReader::from_source(Source::Callback { len, read: Box::new(read) }, ReaderConfig::default())
    }

    /// What the file says about itself.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The component streams of the file from its component block: the ones this reader knows, in file
    /// order, and the ones it stepped over (an unknown capability, or another major version). A file with
    /// no component block gives two empty lists. A frame that is damaged, or that declares a geometry
    /// capability, refuses the whole block.
    pub fn streams(&self) -> Result<(Vec<Stream>, Vec<Skipped>), PvecError> {
        let Some(at) =
            self.inner.entries().iter().position(|e| e.block_kind() == Some(BlockKind::Components))
        else {
            return Ok((Vec::new(), Vec::new()));
        };
        let block = self.inner.read_block(BlockId(at as u32)).map_err(container)?;
        let registry = Registry::v1();
        let reader = Reader {
            version: FORMAT_VERSION,
            implemented: registry.entries().iter().map(|c| c.id).collect(),
        };
        let opened = read_streams(block.as_bytes(), &registry, &reader)
            .map_err(|e| PvecError::Read { rule: e.rule().to_string() })?;
        Ok((opened.streams, opened.skipped))
    }

    /// Reads and checks every block and returns the number of entities in the file.
    pub fn entity_count(&self) -> Result<u64, PvecError> {
        Ok(self.inner.validate().map_err(container)?.entities)
    }

    /// Every entity of the file as a curve, in ascending entity id order.
    ///
    /// A curve comes back as the kind that describes it, not necessarily the kind it was written as:
    /// a Bezier curve is stored as a B-spline and is read as one, and a segment is a trimmed line.
    /// Writing what is read back writes the same bytes. An unbounded line is not storable, and a
    /// composite whose segments are trimmed is refused here for now.
    pub fn entities(&self) -> Result<Vec<(EntityId, Curve)>, PvecError> {
        Ok(self.scan(None, true)?.into_iter().map(|(_, id, curve, _)| (id, curve)).collect())
    }

    /// The style reference of every entity, in ascending entity id order: an opaque index the style
    /// component stream resolves, `0` where the writer gave none.
    pub fn styles(&self) -> Result<Vec<(EntityId, u32)>, PvecError> {
        Ok(self.scan(None, true)?.into_iter().map(|(_, id, _, style)| (id, style)).collect())
    }

    /// Both of the above in one pass over the blocks (conversion layer: hot and advanced records to
    /// curves, with the style column that goes with them), each with the cell of the hot block it came
    /// from (`None` for an advanced curve). `cells` limits the hot blocks to those cells (all when
    /// `None`), and `advanced` says whether the advanced curves are read at all.
    pub(crate) fn scan(
        &self,
        cells: Option<&[u32]>,
        advanced_too: bool,
    ) -> Result<Vec<LoadedEntity>, PvecError> {
        let id = |raw: u32| {
            EntityId::new(raw)
                .map_err(|_| PvecError::Read { rule: "ENTITY-ID: not a valid entity id".to_string() })
        };
        let mut out: Vec<LoadedEntity> = Vec::new();
        for (at, entry) in self.inner.entries().iter().enumerate() {
            // The straddler pages repeat payloads that a leaf's own block holds.
            if entry.block_kind() != Some(BlockKind::Hot) || entry.cell >= STRADDLER_CELL_FIRST {
                continue;
            }
            if cells.is_some_and(|c| !c.contains(&entry.cell)) {
                continue;
            }
            let block = self.inner.read_block(BlockId(at as u32)).map_err(container)?;
            let hot = block.hot().map_err(|e| PvecError::Read { rule: e.rule().to_string() })?;
            let col = |name: &str| hot.column(name).unwrap_or(&[]);
            for (k, raw) in hot.entity_ids().iter().enumerate() {
                let curve = match hot.kind() {
                    HotKind::Line => {
                        let (a, b) = (
                            Point2::new(col("x0")[k], col("y0")[k]),
                            Point2::new(col("x1")[k], col("y1")[k]),
                        );
                        segment(a, b)?
                    }
                    HotKind::Polyline => {
                        let (xs, ys) = hot.polyline(k).unwrap_or((&[], &[]));
                        let points = xs.iter().zip(ys).map(|(x, y)| Point2::new(*x, *y)).collect();
                        Curve::Polyline(Polyline::new(points).map_err(geometry)?)
                    }
                    HotKind::CircleArc => {
                        let frame =
                            Placement2::new(Point2::new(col("cx")[k], col("cy")[k]), Vector2::new(1.0, 0.0))
                                .map_err(geometry)?;
                        let circle = Curve::Circle(Circle::new(frame, col("radius")[k]).map_err(geometry)?);
                        arc(circle, col("start")[k], col("sweep")[k])?
                    }
                    HotKind::EllipseArc => {
                        let frame = Placement2::from_stored(
                            Point2::new(col("cx")[k], col("cy")[k]),
                            axis(col("rotation")[k])?,
                        )
                        .map_err(geometry)?;
                        let ellipse = Curve::Ellipse(
                            Ellipse::new(frame, col("rx")[k], col("ry")[k]).map_err(geometry)?,
                        );
                        arc(ellipse, col("start")[k], col("sweep")[k])?
                    }
                };
                out.push((Some(entry.cell), id(*raw)?, curve, hot.styles()[k]));
            }
        }
        let chunks = if advanced_too { self.inner.advanced_chunk_count().map_err(container)? } else { 0 };
        for chunk in 0..chunks {
            let Some(set) = self.inner.advanced(chunk).map_err(container)? else { continue };
            let view = set.view().map_err(container)?;
            for index in 0..view.len() {
                let Some(record) = view.record(index) else { continue };
                out.push((None, id(record.id)?, advanced(&view, index, 1)?, record.style));
            }
        }
        out.sort_by_key(|(_, id, _, _)| id.index());
        Ok(out)
    }
}
