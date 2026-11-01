//! Viewport streaming, behind the `unstable-stream` feature. It can change without a major version.
//!
//! Besides the re-exports, this file holds what the facade adds on top of the stream crate: an indexed
//! writer, which lays a spatial directory (P5.1) and one Tier 0 block per leaf (P5.4b) into the file, and
//! the reader accessors that give them back.

use crate::writer::refused;
use crate::{Bounds2, CapabilityId, Curve, EntityId, PvecError, PvecReader, PvecWriter};
use pvec_capabilities::{Registry, Version};
use pvec_components::{RASTER_CAPABILITY, encode_rasters};
use pvec_container::Bounds;
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use crate::spill_fs::FsStore;
use pvec_container::spill::{ExternalSorter, SpillError};
use pvec_container::spill_writer::{ByteSink, FinishError, HotRecord, SpillWriter};
use pvec_stream::{
    DirectoryBuilder, DirectoryError, EntityBox, Tier0Builder, Tier0Error, Tier0Outcome, Tier0Params,
    build_directory, entity_key, tier0_blocks, tier0_outcome,
};
use std::collections::HashMap;

pub use crate::hit::{curve_distance, curve_meets_box};
pub use crate::reader::LoadedEntity;
pub use pvec_components::{Band, PixelFormat, Placement, RasterId, TileCodec};
pub use pvec_container::blocks::raster::TileIn;
pub use pvec_container::limits::Limits;
pub use pvec_container::negotiate::{Effect, ReaderCapabilities};
pub use pvec_container::reader::ReaderConfig;
pub use pvec_stream::{
    Aabb, BlockRange, CELL_CAPACITY, CancelToken, Cell, CellId, Fidelity, Generation, GenerationClock,
    InfeasibleReason, PREVIEW_BUDGET_PX, PlannedTile, Preview, PreviewForm, RasterPlan, RasterPlanError,
    ReadPlan, Scene, SpatialDirectory, Tier0Entry, Tier0Leaves, Tier0Plan, TileSlot, ViewportQuery,
    footprint, reported_extent,
};

/// One push, kept so that `finish` can lay the file out again with cells.
#[derive(Debug)]
pub(crate) enum Op {
    Curve(EntityId, Curve),
    Composite { id: EntityId, segments: Vec<EntityId>, bounds: Bounds2, closed: bool },
    Components(crate::components::Stream),
    Raster(RasterId, Vec<TileIn>),
}

/// What an indexed writer remembers.
#[derive(Debug)]
pub(crate) struct Indexing {
    config: WriterConfig,
    capacity: u32,
    tier0_max_error: Option<f64>,
    pub(crate) log: Vec<Op>,
}

fn write_error(rule: &'static str) -> PvecError {
    PvecError::Write { rule: rule.to_string() }
}

fn read_error(rule: &'static str) -> PvecError {
    PvecError::Read { rule: rule.to_string() }
}

impl Indexing {
    /// Builds the directory over the boxes `first` derived, then writes the same pushes again with each hot
    /// entity in its leaf, the directory, and the Tier 0 block of every leaf when a bound was asked for.
    /// A file with no hot entity has nothing to index and is written as it is.
    pub(crate) fn lay_out(self, first: ContainerWriter) -> Result<Vec<u8>, PvecError> {
        let boxes = first.hot_boxes();
        if boxes.is_empty() {
            return first.finish().map_err(refused);
        }
        let entities = boxes
            .iter()
            .map(|(id, b)| {
                Ok(EntityBox { id: *id, bounds: Aabb::new(b.min, b.max).map_err(|e| write_error(e.rule()))? })
            })
            .collect::<Result<Vec<_>, PvecError>>()?;
        let built = build_directory(&entities, self.capacity).map_err(|e| write_error(e.rule()))?;
        let blocks = match self.tier0_max_error {
            None => Vec::new(),
            Some(max_error) => {
                let count = self
                    .log
                    .iter()
                    .filter_map(|op| match op {
                        Op::Curve(id, _) | Op::Composite { id, .. } => Some(id.index() + 1),
                        Op::Components(_) | Op::Raster(..) => None,
                    })
                    .max()
                    .unwrap_or(0);
                let mut plan =
                    Tier0Builder::new(count, Tier0Params { max_error }).map_err(|e| write_error(e.rule()))?;
                for op in &self.log {
                    match op {
                        Op::Curve(id, curve) => plan.add_curve(*id, curve),
                        // A composite is in no leaf, so no leaf block ever carries its entry: the plan only
                        // needs one for every id to be complete, and `tier0_blocks` cuts the leaves' ids.
                        Op::Composite { id, .. } => plan.add_exact(*id),
                        Op::Components(_) | Op::Raster(..) => Ok(()),
                    }
                    .map_err(|e| write_error(e.rule()))?;
                }
                let plan = plan.finish().map_err(|e| write_error(e.rule()))?;
                tier0_blocks(&plan, &entities, &built.cells).map_err(|e| write_error(e.rule()))?
            }
        };
        let cell_of: HashMap<u32, u32> =
            entities.iter().zip(&built.cells).map(|(e, c)| (e.id.index(), c.get())).collect();
        let to_bounds = |a: Aabb| Bounds { min: a.min(), max: a.max() };
        let mut w = ContainerWriter::new(self.config);
        for op in self.log {
            match op {
                Op::Curve(id, curve) => {
                    w.push_curve(id, 0, cell_of.get(&id.index()).copied().unwrap_or(0), &curve)
                }
                Op::Composite { id, segments, bounds, closed } => {
                    let meta = EntityMeta {
                        id,
                        style: 0,
                        cell: 0,
                        bounds: Bounds { min: bounds.min, max: bounds.max },
                    };
                    w.push_composite(meta, &segments, None, closed)
                }
                Op::Components(stream) => w.push_stream(stream),
                Op::Raster(id, tiles) => w.push_raster_tiles(id, tiles),
            }
            .map_err(refused)?;
        }
        w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes).map_err(refused)?;
        for t in blocks {
            w.push_tier0_leaf(t.cell.get(), t.count, to_bounds(t.bounds), t.bytes).map_err(refused)?;
        }
        w.finish().map_err(refused)
    }
}

impl PvecWriter {
    /// Starts a file that carries a spatial directory over its hot entities (segments, polylines, circles,
    /// ellipses and arcs of either), cut into leaves of at most `cell_capacity` entities
    /// ([`CELL_CAPACITY`] is the default size), and, when `tier0_max_error` is a bound in world units, one
    /// Tier 0 block per leaf whose previews stay within it (`PREVIEW_BUDGET_PX * extent / 1920` is the
    /// budget of the benchmarks). Entities pushed afterwards are placed when [`PvecWriter::finish`] runs, so
    /// an error that only the index can raise (a capacity of 0, a Tier 0 bound that is not finite and above
    /// zero) shows there. Splines and composites are the advanced curves: they stay in no leaf. The
    /// writer keeps every push until `finish`, which is the cost of placing them (use [`PvecWriter::with_spill`] to keep them in files instead). A Tier 0 bound needs the entity ids to be dense, 0 to n-1 with none missing or repeated, or `finish` refuses with the Tier 0 coverage rule (`TIER0-01`). The result is
    /// deterministic, like the plain writer's.
    pub fn new_indexed(
        manifest: crate::Manifest,
        cell_capacity: u32,
        tier0_max_error: Option<f64>,
    ) -> Result<PvecWriter, PvecError> {
        let config = manifest.config()?;
        let mut writer = PvecWriter::new(manifest)?;
        writer.index = Some(Indexing { config, capacity: cell_capacity, tier0_max_error, log: Vec::new() });
        Ok(writer)
    }
}

impl PvecReader {
    /// The spatial directory of the file, `None` when it has none. A directory the reader refuses is an
    /// error here and nowhere else: the entities do not depend on it.
    pub fn spatial_directory(&self) -> Result<Option<SpatialDirectory>, PvecError> {
        SpatialDirectory::open(&self.inner).map_err(|e| read_error(e.rule()))
    }

    /// The Tier 0 blocks of the file's leaves, checked against `directory`. A file with none gives an empty set.
    pub fn tier0_leaves(&self, directory: &SpatialDirectory) -> Result<Tier0Leaves, PvecError> {
        Tier0Leaves::new(self.inner.entries(), directory).map_err(|e| read_error(e.rule()))
    }

    /// The Tier 0 plan of one leaf: one block read. `None` when the leaf has no block.
    pub fn tier0_plan(&self, leaves: &Tier0Leaves, leaf: CellId) -> Result<Option<Tier0Plan>, PvecError> {
        leaves.tier0(&self.inner, leaf).map_err(|e| read_error(e.rule()))
    }

    /// The bytes the blocks of `blocks` take in the file as stored, which is what a range request for
    /// them costs.
    pub fn stored_bytes(&self, blocks: BlockRange) -> u64 {
        let first = blocks.first.0 as usize;
        self.inner
            .entries()
            .get(first..first.saturating_add(blocks.count as usize))
            .map_or(0, |run| run.iter().map(|e| u64::from(e.stored_length)).sum())
    }
}

impl PvecReader {
    /// The byte ranges that reading the Tier 0 blocks of `cells` costs, as `(offset, length)`: blocks close
    /// together in one range, the way the reader asks for them, and a block it already holds left out. Nothing
    /// is read. A host that serves the file by ranges (`http`) fetches these before it makes the call.
    pub fn tier0_ranges(&self, leaves: &Tier0Leaves, cells: &[CellId]) -> Result<Vec<(u64, u32)>, PvecError> {
        let ids: Vec<_> = cells.iter().filter_map(|c| leaves.block(*c)).collect();
        self.ranges_of(&ids)
    }

    /// The byte ranges that [`PvecReader::entities_in`] would read, as above.
    pub fn entity_ranges(&self, cells: Option<&[u32]>, advanced: bool) -> Result<Vec<(u64, u32)>, PvecError> {
        let mut ids: Vec<_> = (0..self.inner.entries().len() as u32)
            .map(pvec_container::reader::BlockId)
            .filter(|id| {
                let e = &self.inner.entries()[id.0 as usize];
                e.block_kind() == Some(pvec_container::directory::BlockKind::Hot)
                    && e.cell < pvec_container::directory::STRADDLER_CELL_FIRST
                    && cells.is_none_or(|c| c.contains(&e.cell))
            })
            .collect();
        if advanced {
            let chunks = self.inner.advanced_chunk_count().map_err(|e| PvecError::Read { rule: e.rule() })?;
            let all: Vec<usize> = (0..chunks).collect();
            ids.extend(
                self.inner.advanced_chunk_blocks(&all).map_err(|e| PvecError::Read { rule: e.rule() })?,
            );
        }
        self.ranges_of(&ids)
    }

    fn ranges_of(&self, ids: &[pvec_container::reader::BlockId]) -> Result<Vec<(u64, u32)>, PvecError> {
        let missing: Vec<_> = ids.iter().copied().filter(|id| !self.inner.is_cached(*id)).collect();
        let plan = self.inner.plan(&missing).map_err(|e| PvecError::Read { rule: e.rule() })?;
        Ok(plan.into_iter().map(|r| (r.offset, r.len)).collect())
    }

    /// The entities of the hot blocks of `cells` (all of them when `None`) and, when `advanced`, of the
    /// advanced curves, each with the cell it was read from (`None` for an advanced curve) and its style
    /// reference, in ascending entity id order. It reads only those blocks, which is what makes a
    /// viewport cost its own leaves when the file is behind ranges.
    pub fn entities_in(&self, cells: Option<&[u32]>, advanced: bool) -> Result<Vec<LoadedEntity>, PvecError> {
        self.scan(cells, advanced)
    }
}

impl PvecReader {
    /// Opens a file with `config`: the capabilities the reader implements, the limits it holds the file to
    /// and the rest of the container's reader configuration. This is how a host checks what an older build
    /// would show (a capability the reader lacks is skipped and not refused when the file declares it
    /// optional, see [`PvecReader::skipped_effects`]) and how it applies the browser limits.
    pub fn open_with_config(bytes: Vec<u8>, config: ReaderConfig) -> Result<PvecReader, PvecError> {
        PvecReader::from_source(crate::reader::Source::Memory(MemorySource::new(bytes)), config)
    }

    /// [`PvecReader::open_ranges`] with `config`, which is how a host that serves byte ranges applies the
    /// browser limits. Behind the `http` feature.
    #[cfg(feature = "http")]
    pub fn open_ranges_with_config(
        len: u64,
        read: impl Fn(u64, u32) -> Result<Vec<u8>, String> + Send + Sync + 'static,
        config: ReaderConfig,
    ) -> Result<PvecReader, PvecError> {
        PvecReader::from_source(crate::reader::Source::Callback { len, read: Box::new(read) }, config)
    }

    /// What the open left undone, one entry per skipped capability in ascending id order, or nothing when
    /// the file was read whole. A viewer turns `Effect::Unrastered` into "this file has an image your reader
    /// cannot show".
    pub fn skipped_effects(&self) -> Vec<(CapabilityId, Effect)> {
        self.inner
            .partial_open()
            .map(|p| p.capabilities.iter().map(|c| (c.id, c.effect)).collect())
            .unwrap_or_default()
    }
}

impl PvecWriter {
    /// Attaches the raster stream (capability 21): the placements, in `RasterId` order, that the tiles of
    /// [`PvecWriter::raster_tiles`] are placed by. A placement the rules refuse (`RASTER-01`, `04`, `05`, `07`,
    /// `LIMIT-13`) is an error here and not at `finish`.
    pub fn rasters(&mut self, placements: &[Placement]) -> Result<(), PvecError> {
        let payload = encode_rasters(placements).map_err(|e| write_error(e.rule()))?;
        let version = Registry::v1().get(RASTER_CAPABILITY).map_or(Version::new(1, 0, 0), |c| c.version);
        self.components(crate::components::Stream { capability: RASTER_CAPABILITY, version, payload })
    }

    /// Adds the encoded tiles of raster `raster` (PNG, JPEG or WebP, stored as they are), each with its level,
    /// column and row. A tile with no bytes is a tile that is not stored. The tiles of one raster can come in
    /// any number of calls and any order, and the file does not depend on either. What the container cannot
    /// store is refused by `finish`, with the rule of the refusal.
    pub fn raster_tiles(&mut self, raster: RasterId, tiles: Vec<TileIn>) -> Result<(), PvecError> {
        self.touched = true;
        if let Some(spill) = &mut self.spill {
            return spill.push_raster_tiles(raster, tiles);
        }
        if let Some(index) = &mut self.index {
            index.log.push(Op::Raster(raster, tiles.clone()));
        }
        self.inner.push_raster_tiles(raster, tiles).map_err(refused)
    }
}

impl PvecReader {
    /// The raster placements of the file, in `RasterId` order: empty when the file has none, and an error when
    /// the stream is damaged (the geometry does not depend on it).
    pub fn raster_placements(&self) -> Result<Vec<Placement>, PvecError> {
        self.inner.raster_placements().map(|p| p.to_vec()).map_err(|e| PvecError::Read { rule: e.rule() })
    }

    /// The encoded bytes of tile `(col, row)` of `level` of raster `raster`, after the index's checksum has been
    /// checked against them: `None` for a tile that is not stored, and an error for a raster the file does not
    /// have or one it refuses (the file and the other rasters are unaffected).
    pub fn raster_tile(
        &self,
        raster: RasterId,
        level: u32,
        col: u32,
        row: u32,
    ) -> Result<Option<Vec<u8>>, PvecError> {
        self.inner
            .read_raster_tile(raster.index(), level, col, row)
            .map(|t| t.map(|b| b.as_slice().to_vec()))
            .map_err(|e| PvecError::Read { rule: e.rule() })
    }
}

impl PvecReader {
    /// The rasters the directory names, ascending.
    pub fn rasters(&self) -> Vec<u32> {
        self.inner.rasters()
    }

    /// Reads the index block of `raster` and keeps it, which a plan or a tile call then finds. On a file behind
    /// ranges it is the call that names the block's range; a host that plans for several rasters loads each first
    /// so that every missing range is named at once.
    pub fn raster_load(&self, raster: u32) -> Result<(), PvecError> {
        self.inner.raster_index(raster).map(drop).map_err(crate::reader::container)
    }

    /// The tiles of `raster` that `query` needs: `pvec_stream::plan_placed` over this reader's directory and
    /// limits, with `LIMIT-16` replaced by `max_decoded` when given. It reads the index block when it has not
    /// (see [`PvecReader::raster_load`]) and no tile.
    pub fn raster_plan(
        &self,
        raster: u32,
        placement: &Placement,
        query: &ViewportQuery,
        max_decoded: Option<u64>,
    ) -> Result<RasterPlan, PvecError> {
        let index = self.inner.raster_index(raster).map_err(crate::reader::container)?;
        let mut config = self.inner.config().clone();
        if let Some(max) = max_decoded {
            config.limits.max_plan_decoded_bytes = max;
        }
        pvec_stream::plan_placed(&self.inner, raster, placement, &index, query, &config).map_err(|e| {
            read_error(match e {
                RasterPlanError::OverBudget => "LIMIT-16",
                RasterPlanError::Skipped => "RASTER-04",
                _ => "RASTER-05",
            })
        })
    }

    /// Where tile `(level, col, row)` of `raster` is: the range of its chunk a host would fetch, inline in the
    /// index block, or not stored. `None` for an address outside the pyramid.
    pub fn raster_tile_slot(
        &self,
        raster: u32,
        level: u32,
        col: u32,
        row: u32,
    ) -> Result<Option<TileSlot>, PvecError> {
        let index = self.inner.raster_index(raster).map_err(crate::reader::container)?;
        Ok(index
            .tile(level, col, row)
            .map(|_| pvec_stream::tile_slots(&self.inner, raster, &index)(level, col, row)))
    }
}

// ------------------------------------------------------------------------------------ bounded memory

fn io_error(stage: &'static str, e: std::io::Error) -> PvecError {
    PvecError::Io { stage, error: e.into() }
}

/// A failure of the store or the sink: the `std::io::Error` the file system raised, or, for a file that does not hold
/// what the writer put there, one of kind `InvalidData`.
fn spill_error(stage: &'static str, e: SpillError) -> PvecError {
    let source = e.into_source();
    io_error(
        stage,
        match source.downcast::<std::io::Error>() {
            Ok(e) => *e,
            Err(other) => std::io::Error::new(std::io::ErrorKind::InvalidData, other),
        },
    )
}

fn finish_error(e: FinishError) -> PvecError {
    match e {
        FinishError::Write(w) => refused(w),
        FinishError::Spill(e) => spill_error("spill", e),
        FinishError::Output(e) => spill_error("output", e),
        other => PvecError::Write { rule: other.to_string() },
    }
}

/// An `io::Write` as the sink the container writes the artifact to.
struct IoSink<'a, W: std::io::Write>(&'a mut W);

impl<W: std::io::Write> ByteSink for IoSink<'_, W> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), SpillError> {
        self.0.write_all(bytes).map_err(SpillError::new)
    }
}

fn tier0_error(e: Tier0Error) -> PvecError {
    write_error(e.rule())
}

fn directory_error(e: DirectoryError) -> PvecError {
    write_error(e.rule())
}

/// What a writer that keeps its pushes in files holds besides the files.
#[derive(Debug)]
pub(crate) struct Spill {
    writer: SpillWriter,
    /// The bound the previews of Tier 0 are held to, when the writer is indexed and was asked for Tier 0.
    tier0: Option<Tier0Params>,
    /// The first error that classifying a pushed curve for Tier 0 gave, which `finish_to` reports where the
    /// in-memory writer reports it.
    classify: Option<Tier0Error>,
}

impl Spill {
    pub(crate) fn push(&mut self, id: EntityId, curve: &Curve) -> Result<(), PvecError> {
        let Spill { writer, tier0, classify } = self;
        let mut tag = || match tier0 {
            Some(params) => match tier0_outcome(id, curve, params) {
                Ok(outcome) => outcome.to_bytes(),
                Err(e) => {
                    classify.get_or_insert(e);
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        writer.push_curve(id, 0, 0, curve, &mut tag).map_err(finish_error)
    }

    pub(crate) fn push_composite(
        &mut self,
        meta: EntityMeta,
        segments: &[EntityId],
        closed: bool,
    ) -> Result<(), PvecError> {
        self.writer.push_composite(meta, segments, None, closed).map_err(finish_error)
    }

    pub(crate) fn push_stream(&mut self, stream: &crate::components::Stream) -> Result<(), PvecError> {
        self.writer.push_stream(stream).map_err(finish_error)
    }

    pub(crate) fn push_raster_tiles(&mut self, raster: RasterId, tiles: Vec<TileIn>) -> Result<(), PvecError> {
        self.writer.push_raster_tiles(raster, tiles).map_err(finish_error)
    }
}

impl PvecWriter {
    /// Makes the writer keep its pushes in files under `root` and build the file from them, inside about
    /// `budget` bytes of memory, plus the entities of the biggest block it has to build (a leaf of an indexed
    /// writer, or every entity of one geometry type when the writer is not indexed: use
    /// [`PvecWriter::new_indexed`] for a bound that does not grow with the drawing).
    ///
    /// `root` must be a directory that exists. The writer makes one directory in it for its files, and removes
    /// it when it finishes, fails or is dropped; nothing is written next to the output or anywhere else. It has to
    /// be called before the first push (`Write` error `SPILL-01` after), and an error making the directory is an
    /// [`PvecError::Io`] with stage `"spill"`. [`PvecWriter::finish_to`] is where the file is built; the file is
    /// the one the writer would have returned without this call, byte for byte, whatever the budget.
    pub fn with_spill(mut self, root: &std::path::Path, budget: u64) -> Result<PvecWriter, PvecError> {
        if self.touched || self.spill.is_some() {
            return Err(write_error("SPILL-01"));
        }
        let budget = usize::try_from(budget).unwrap_or(usize::MAX);
        let store = FsStore::create(root).map_err(|e| io_error("spill", e))?;
        let writer = SpillWriter::new(self.config.clone(), std::sync::Arc::new(store), budget)
            .map_err(|e| spill_error("spill", e))?;
        let tier0 = self.index.as_ref().and_then(|i| i.tier0_max_error).map(|max_error| Tier0Params { max_error });
        self.spill = Some(Box::new(Spill { writer, tier0, classify: None }));
        Ok(self)
    }

    /// Seals the file and writes it to `out`, returning the number of bytes written. `out` needs no `Seek`, and
    /// receives nothing unless the whole file was built: an error from the build (a rule, or a spill file that
    /// cannot be read or written) leaves it untouched, and an error from `out` itself comes back as
    /// [`PvecError::Io`] with stage `"output"` and the `std::io::Error` as its source.
    ///
    /// On a writer made with [`PvecWriter::with_spill`] the build stays inside its budget. On any other it is
    /// [`PvecWriter::finish`] and one write.
    pub fn finish_to(self, out: &mut impl std::io::Write) -> Result<u64, PvecError> {
        if self.spill.is_none() {
            let bytes = self.finish()?;
            out.write_all(&bytes).map_err(|e| io_error("output", e))?;
            out.flush().map_err(|e| io_error("output", e))?;
            return Ok(bytes.len() as u64);
        }
        let Some(spill) = self.spill else { unreachable!("checked above") };
        let Spill { writer, tier0, classify } = *spill;
        let mut sink = IoSink(out);
        let written = match self.index {
            None => writer.finish_to(&mut sink).map_err(finish_error),
            Some(index) => index.lay_out_spilled(writer, tier0, classify, &mut sink),
        }?;
        out.flush().map_err(|e| io_error("output", e))?;
        Ok(written)
    }
}

impl Indexing {
    /// [`Indexing::lay_out`] over a writer whose pushes are in files: the same directory, the same cells, the
    /// same Tier 0 blocks, built from an external sort of the hot entities by the key the directory orders by.
    fn lay_out_spilled(
        self,
        mut w: SpillWriter,
        tier0: Option<Tier0Params>,
        classify: Option<Tier0Error>,
        out: &mut dyn ByteSink,
    ) -> Result<u64, PvecError> {
        if w.hot_count() == 0 {
            return w.finish_to(out).map_err(finish_error);
        }
        let build = |reason| directory_error(DirectoryError::Build { reason });
        let spill = |e| spill_error("spill", e);
        if self.capacity == 0 {
            return Err(build("a leaf holds no entity"));
        }
        let (store, budget) = (w.store(), w.budget());
        // the first pass: the box of the drawing, and whether a hot id is used twice
        let mut world: Option<Aabb> = None;
        let mut ids = ExternalSorter::new(&store, budget / 4);
        let mut hot = 0usize;
        for rec in w.hot_records().map_err(spill)? {
            let rec = rec.map_err(spill)?;
            let m = rec.meta();
            let b = Aabb::new(m.bounds.min, m.bounds.max).map_err(|e| write_error(e.rule()))?;
            world = Some(world.map_or(b, |x| x.union(&b)));
            ids.push(u128::from(m.id.index()), Vec::new()).map_err(spill)?;
            hot += 1;
        }
        if hot > pvec_numeric::MAX_ENTITIES {
            return Err(build("more entities than NP-18 allows"));
        }
        let mut previous = None;
        for rec in ids.finish().map_err(spill)? {
            let (id, _) = rec.map_err(spill)?;
            if previous == Some(id) {
                return Err(build("an entity id is used twice"));
            }
            previous = Some(id);
        }
        let Some(world) = world else { return w.finish_to(out).map_err(finish_error) };
        // Tier 0 covers every id, so the ids must be dense; its refusals come before anything is built
        let summary = w.id_summary().map_err(finish_error)?;
        let count = summary.max.map_or(0, |m| m + 1);
        if let Some(params) = tier0 {
            Tier0Builder::new(count, params).map_err(tier0_error)?;
            if let Some(e) = classify {
                return Err(tier0_error(e));
            }
            let entity = |i: u32| EntityId::new(i).map_err(|_| tier0_error(Tier0Error::Limit));
            if let Some(i) = summary.duplicate {
                return Err(tier0_error(Tier0Error::Duplicate(entity(i)?)));
            }
            if let Some(i) = summary.first_gap {
                return Err(tier0_error(Tier0Error::Gap(entity(i)?)));
            }
        }
        // the second pass: the hot entities in the order the directory cuts them into cells
        let mut sorter = ExternalSorter::new(&store, budget);
        for rec in w.hot_records().map_err(spill)? {
            let rec = rec.map_err(spill)?;
            let m = rec.meta();
            let b = Aabb::new(m.bounds.min, m.bounds.max).map_err(|e| write_error(e.rule()))?;
            let key = u128::from(entity_key(&world, &b)) << 32 | u128::from(m.id.index());
            sorter.push(key, rec.encode()).map_err(spill)?;
        }
        let mut cells = DirectoryBuilder::new(self.capacity).map_err(directory_error)?;
        let mut group = Vec::new();
        let mut open: Option<(u32, Aabb)> = None;
        for item in sorter.finish().map_err(spill)? {
            let (_, bytes) = item.map_err(spill)?;
            let rec = HotRecord::decode(&bytes).map_err(spill)?;
            let m = rec.meta();
            let b = Aabb::new(m.bounds.min, m.bounds.max).map_err(|e| write_error(e.rule()))?;
            let cell = cells.push(&b).map_err(directory_error)?.get();
            match &mut open {
                Some((c, bounds)) if *c == cell => *bounds = bounds.union(&b),
                _ => {
                    if let Some((c, bounds)) = open.take() {
                        place_cell(&mut w, c, bounds, std::mem::take(&mut group), tier0.is_some(), count)?;
                    }
                    open = Some((cell, b));
                }
            }
            group.push(rec);
        }
        if let Some((c, bounds)) = open.take() {
            place_cell(&mut w, c, bounds, group, tier0.is_some(), count)?;
        }
        let built = cells.finish().map_err(directory_error)?;
        let bounds = Bounds { min: built.bounds.min(), max: built.bounds.max() };
        w.set_spatial_directory(built.nodes, bounds, built.bytes).map_err(refused)?;
        w.finish_to(out).map_err(finish_error)
    }
}

/// Puts the entities of one cell in their blocks and, when Tier 0 was asked for, writes the cell's Tier 0 block.
fn place_cell(
    w: &mut SpillWriter,
    cell: u32,
    bounds: Aabb,
    records: Vec<HotRecord>,
    tier0: bool,
    count: u32,
) -> Result<(), PvecError> {
    if tier0 {
        let mut outcomes = Vec::with_capacity(records.len());
        for r in &records {
            let id = r.meta().id;
            outcomes.push((id, Tier0Outcome::from_bytes(id, &r.tag).map_err(tier0_error)?));
        }
        let bytes = Tier0Plan::leaf(count, outcomes).map_err(tier0_error)?.encode();
        let to = Bounds { min: bounds.min(), max: bounds.max() };
        w.push_tier0_leaf(cell, records.len() as u32, to, bytes).map_err(finish_error)?;
    }
    w.emit_hot_cell(cell, records).map_err(finish_error)
}
