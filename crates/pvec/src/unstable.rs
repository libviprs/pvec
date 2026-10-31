//! Viewport streaming, behind the `unstable-stream` feature. It can change without a major version.
//!
//! Besides the re-exports, this file holds what the facade adds on top of the stream crate: an indexed
//! writer, which lays a spatial directory (P5.1) and one Tier 0 block per leaf (P5.4b) into the file, and
//! the reader accessors that give them back.

use crate::writer::refused;
use crate::{Bounds2, CapabilityId, Curve, EntityId, PvecError, PvecReader, PvecWriter};
use pvec_container::Bounds;
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_stream::{EntityBox, Tier0Builder, Tier0Params, build_directory, tier0_blocks};
use std::collections::HashMap;

pub use crate::reader::LoadedEntity;
pub use pvec_container::limits::Limits;
pub use pvec_container::negotiate::{Effect, ReaderCapabilities};
pub use pvec_container::reader::ReaderConfig;
pub use pvec_stream::{
    Aabb, BlockRange, CELL_CAPACITY, CancelToken, Cell, CellId, Fidelity, Generation, GenerationClock,
    InfeasibleReason, PREVIEW_BUDGET_PX, Preview, PreviewForm, ReadPlan, Scene, SpatialDirectory, Tier0Entry,
    Tier0Leaves, Tier0Plan, ViewportQuery,
};

/// One push, kept so that `finish` can lay the file out again with cells.
#[derive(Debug)]
pub(crate) enum Op {
    Curve(EntityId, Curve),
    Composite { id: EntityId, segments: Vec<EntityId>, bounds: Bounds2, closed: bool },
    Components(crate::components::Stream),
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
                        Op::Components(_) => None,
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
                        Op::Components(_) => Ok(()),
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
    /// writer keeps every push until `finish`, which is the cost of placing them. The result is
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
