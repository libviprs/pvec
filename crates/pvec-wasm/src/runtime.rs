use crate::abi::{
    TIER0_BOUND_NOT_MET, TIER0_CHORD_POLYLINE, TIER0_CONTROL_POLYGON, TIER0_EVALUATOR_REFUSED, TIER0_EXACT,
    TIER0_INFEASIBLE_OTHER, TIER0_VERTEX_CAP,
};
use crate::{AbiError, BufferKind, DISPLAY_FIDELITY, ElemType, Precision, View};
use pvec::unstable::{Aabb, InfeasibleReason, PreviewForm, SpatialDirectory, Tier0Leaves};
use pvec::{Bounds2, Curve, CurveEval, EntityId, PvecError, PvecReader, Tessellate, TessellationContext};
use std::rc::Rc;

/// One entity of an open file.
struct Entity {
    id: EntityId,
    curve: Curve,
    /// `None` for a curve that has no bounds (an unbounded line), which no viewport can cull.
    bounds: Option<Bounds2>,
    style: u32,
}

/// The spatial directory and the Tier 0 blocks of a file that has them, with the reader that reads a leaf's
/// block when a call asks for it (so the file's bytes stay in memory as long as the file is open).
struct Index {
    reader: PvecReader,
    dir: SpatialDirectory,
    leaves: Tier0Leaves,
}

/// What an open file keeps: the entities, in ascending id order, and its index. A batch shares it, so
/// releasing the file does not pull the curves out from under a batch. A directory the reader refused is
/// kept as the error, so that only the calls that need it fail.
struct File {
    entities: Vec<Entity>,
    index: Result<Option<Index>, AbiError>,
}

/// What the spatial directory of a file says about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectoryInfo {
    /// Leaves.
    pub cells: u32,
    /// Levels of the tree; 1 for a single leaf.
    pub height: u32,
    /// Hot entities the leaves hold.
    pub entities: u32,
    /// Leaves that have a Tier 0 block.
    pub tier0_leaves: u32,
    /// The box of the whole tree.
    pub bounds: Bounds2,
    /// The box of the advanced curves (splines and composites), which no leaf holds.
    pub advanced_bounds: Option<Bounds2>,
}

/// The leaves a window touches, as parallel arrays (a `directory_query` result). Entry `i` of each of
/// `ids`, `entities`, `has_tier0` and `stored_bytes` is leaf `i`; `bounds` has four numbers per leaf.
#[derive(Clone, Debug, PartialEq)]
pub struct CellList {
    /// The cell ids, ascending.
    pub ids: Vec<u32>,
    /// Hot entities in each leaf.
    pub entities: Vec<u32>,
    /// 1 when the leaf has a Tier 0 block, else 0.
    pub has_tier0: Vec<u32>,
    /// The stored bytes of each leaf's hot blocks.
    pub stored_bytes: Vec<f64>,
    /// `min_x, min_y, max_x, max_y` of each leaf.
    pub bounds: Vec<f64>,
}

/// The Tier 0 stream of a window (a `tier0_viewport` result): one entry per hot entity of the window,
/// ascending by entity id, as parallel arrays, and the counts that make the coverage auditable.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier0Stream {
    /// The entity of each entry.
    pub ids: Vec<u32>,
    /// `TIER0_*` code of each entry.
    pub codes: Vec<u32>,
    /// The declared error of each entry in world units: 0 for an exact entry, the stored bound for a
    /// preview, NaN for an entity declared infeasible.
    pub error_bounds: Vec<f64>,
    /// `entries + 1` indices into `vertices` (points, not numbers); a preview owns a range, any other entry an empty one.
    pub offsets: Vec<u32>,
    /// Interleaved `x, y` of the previews, in world space, after the stored `f32` quantisation.
    pub vertices: Vec<f64>,
    /// Entries that go out exact.
    pub exact: u32,
    /// Entries that are previews.
    pub previews: u32,
    /// Entries declared infeasible.
    pub infeasible: u32,
    /// Entities of the window that no entry covers: the advanced curves, and the hot entities of a leaf with no block.
    pub uncovered: u32,
    /// Leaves of the window whose block was read.
    pub leaves_read: u32,
    /// Leaves of the window with no Tier 0 block.
    pub leaves_without_block: u32,
    /// 1 when the bounds of the advanced curves meet the window.
    pub advanced_in_view: u32,
}

fn meets(b: &Bounds2, q: &Bounds2) -> bool {
    b.min[0] <= q.max[0] && b.max[0] >= q.min[0] && b.min[1] <= q.max[1] && b.max[1] >= q.min[1]
}

enum Vertices {
    F64(Vec<f64>),
    F32(Vec<f32>),
}

/// The four arrays of a result, and the scalars that go with them.
struct Packed {
    vertices: Vertices,
    offsets: Vec<u32>,
    ids: Vec<u32>,
    styles: Vec<u32>,
    origin: [f64; 2],
    failed: u32,
}

enum Item {
    File(Rc<File>),
    Batch { file: Rc<File>, picked: Vec<usize>, truncated: bool },
    Tessellation(Packed),
    Cells(CellList),
    Tier0(Tier0Stream),
}

struct Slot {
    generation: u16,
    item: Option<Item>,
}

/// Every handle, every batch and every packed result of one module instance.
///
/// A handle is an index and a generation in one `u32`, so a handle that was released stays refused after
/// its slot is reused. Nothing here is shared between threads: a module instance is one thread.
#[derive(Default)]
pub struct Runtime {
    slots: Vec<Slot>,
    /// Stands in for the size of linear memory off `wasm32`.
    #[cfg(not(target_arch = "wasm32"))]
    grown: u32,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Runtime {{ {} slots }}", self.slots.len())
    }
}

const INDEX_BITS: u32 = 16;

impl Runtime {
    /// A runtime with nothing open.
    pub fn new() -> Runtime {
        Runtime::default()
    }

    fn insert(&mut self, item: Item) -> u32 {
        let at = match self.slots.iter().position(|s| s.item.is_none()) {
            Some(at) => at,
            None => {
                self.slots.push(Slot { generation: 0, item: None });
                self.slots.len() - 1
            }
        };
        let slot = &mut self.slots[at];
        slot.item = Some(item);
        (u32::from(slot.generation) << INDEX_BITS) | at as u32
    }

    fn get(&self, handle: u32) -> Result<&Item, AbiError> {
        let slot = self.slots.get((handle & ((1 << INDEX_BITS) - 1)) as usize).ok_or(AbiError::Released)?;
        match &slot.item {
            Some(item) if u32::from(slot.generation) == handle >> INDEX_BITS => Ok(item),
            _ => Err(AbiError::Released),
        }
    }

    fn packed(&self, handle: u32) -> Result<&Packed, AbiError> {
        match self.get(handle)? {
            Item::Tessellation(p) => Ok(p),
            _ => Err(AbiError::WrongKind),
        }
    }

    /// Opens a PVec file held in memory and returns its handle.
    pub fn open(&mut self, bytes: Vec<u8>) -> Result<u32, AbiError> {
        let read = |e: PvecError| {
            let _ = e;
            AbiError::Read
        };
        let reader = PvecReader::open(bytes).map_err(read)?;
        let styles = reader.styles().map_err(read)?;
        let entities = reader
            .entities()
            .map_err(read)?
            .into_iter()
            .zip(styles)
            .map(|((id, curve), (_, style))| Entity { id, bounds: curve.bounds().ok(), curve, style })
            .collect();
        let index = match reader.spatial_directory() {
            Ok(Some(dir)) => match reader.tier0_leaves(&dir) {
                Ok(leaves) => Ok(Some(Index { reader, dir, leaves })),
                Err(_) => Err(AbiError::Read),
            },
            Ok(None) => Ok(None),
            Err(_) => Err(AbiError::Read),
        };
        Ok(self.insert(Item::File(Rc::new(File { entities, index }))))
    }

    /// The entities of `file` whose bounds meet `bounds`, at most `budget` of them in ascending id
    /// order, as a batch handle.
    pub fn query_viewport(&mut self, file: u32, bounds: Bounds2, budget: u32) -> Result<u32, AbiError> {
        let file = match self.get(file)? {
            Item::File(f) => Rc::clone(f),
            _ => return Err(AbiError::WrongKind),
        };
        if bounds.min.iter().chain(&bounds.max).any(|v| v.is_nan()) {
            return Err(AbiError::BadArgument);
        }
        let mut picked: Vec<usize> = (0..file.entities.len())
            .filter(|i| file.entities[*i].bounds.as_ref().is_none_or(|b| meets(b, &bounds)))
            .collect();
        let truncated = picked.len() > budget as usize;
        picked.truncate(budget as usize);
        Ok(self.insert(Item::Batch { file, picked, truncated }))
    }

    fn file(&self, handle: u32) -> Result<Rc<File>, AbiError> {
        match self.get(handle)? {
            Item::File(f) => Ok(Rc::clone(f)),
            _ => Err(AbiError::WrongKind),
        }
    }

    /// What the spatial directory of `file` says, `None` for a file that has none. A directory the reader
    /// refused when the file was opened is [`AbiError::Read`] here.
    pub fn directory(&self, file: u32) -> Result<Option<DirectoryInfo>, AbiError> {
        let file = self.file(file)?;
        let Some(index) = file.index.as_ref().map_err(|e| *e)? else { return Ok(None) };
        let boxed = |a: Aabb| Bounds2 { min: a.min(), max: a.max() };
        let count = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(Some(DirectoryInfo {
            cells: count(index.dir.cells() as u64),
            height: index.dir.height(),
            entities: count(index.dir.entities()),
            tier0_leaves: count(index.leaves.len() as u64),
            bounds: boxed(index.dir.bounds()),
            advanced_bounds: index.dir.advanced_bounds().map(boxed),
        }))
    }

    /// The leaves whose bounds touch `bounds`, in ascending cell id, as a handle.
    pub fn directory_query(&mut self, file: u32, bounds: Bounds2) -> Result<u32, AbiError> {
        let file = self.file(file)?;
        let index = file.index.as_ref().map_err(|e| *e)?.as_ref().ok_or(AbiError::NoIndex)?;
        let q = Aabb::new(bounds.min, bounds.max).map_err(|_| AbiError::BadArgument)?;
        let mut list = CellList {
            ids: Vec::new(),
            entities: Vec::new(),
            has_tier0: Vec::new(),
            stored_bytes: Vec::new(),
            bounds: Vec::new(),
        };
        for cell in index.dir.query(&q) {
            list.ids.push(cell.id.get());
            list.entities.push(cell.entities);
            list.has_tier0.push(u32::from(index.leaves.block(cell.id).is_some()));
            list.stored_bytes.push(index.reader.stored_bytes(cell.blocks) as f64);
            list.bounds.extend([
                cell.bounds.min()[0],
                cell.bounds.min()[1],
                cell.bounds.max()[0],
                cell.bounds.max()[1],
            ]);
        }
        Ok(self.insert(Item::Cells(list)))
    }

    /// The cells of a `directory_query` result.
    pub fn cells(&self, result: u32) -> Result<&CellList, AbiError> {
        match self.get(result)? {
            Item::Cells(c) => Ok(c),
            _ => Err(AbiError::WrongKind),
        }
    }

    /// Array `which` of a cells result: 0 ids, 1 entities, 2 has Tier 0 (`Uint32Array`), 3 stored bytes,
    /// 4 bounds (`Float64Array`).
    pub fn cells_view(&self, result: u32, which: u32) -> Result<View, AbiError> {
        let c = self.cells(result)?;
        match which {
            0 => Ok(view_u32(&c.ids)),
            1 => Ok(view_u32(&c.entities)),
            2 => Ok(view_u32(&c.has_tier0)),
            3 => Ok(view_f64(&c.stored_bytes)),
            4 => Ok(view_f64(&c.bounds)),
            _ => Err(AbiError::BadArgument),
        }
    }

    /// The Tier 0 stream of the window: for each hot entity of the leaves it touches (and whose own bounds
    /// meet it), whether it goes out exact, as a preview with its declared error, or declared infeasible,
    /// and how many entities of the window no entry covers.
    pub fn tier0_viewport(&mut self, file: u32, bounds: Bounds2) -> Result<u32, AbiError> {
        let file = self.file(file)?;
        let index = file.index.as_ref().map_err(|e| *e)?.as_ref().ok_or(AbiError::NoIndex)?;
        let q = Aabb::new(bounds.min, bounds.max).map_err(|_| AbiError::BadArgument)?;
        let mut rows: Vec<(u32, u32, f64, Vec<[f64; 2]>)> = Vec::new();
        let (mut leaves_read, mut leaves_without_block) = (0, 0);
        for cell in index.dir.query(&q) {
            let plan = index.reader.tier0_plan(&index.leaves, cell.id).map_err(|_| AbiError::Read)?;
            let Some(plan) = plan else {
                leaves_without_block += 1;
                continue;
            };
            leaves_read += 1;
            rows.extend(plan.exact_ids().map(|id| (id.index(), TIER0_EXACT, 0.0, Vec::new())));
            rows.extend(plan.previews().iter().map(|p| {
                let code = match p.form() {
                    PreviewForm::ControlPolygon => TIER0_CONTROL_POLYGON,
                    PreviewForm::ChordPolyline => TIER0_CHORD_POLYLINE,
                };
                (p.replaces().index(), code, p.error_bound(), p.points())
            }));
            rows.extend(plan.infeasible().iter().map(|i| {
                let code = match i.reason {
                    InfeasibleReason::EvaluatorRefused => TIER0_EVALUATOR_REFUSED,
                    InfeasibleReason::VertexCap => TIER0_VERTEX_CAP,
                    InfeasibleReason::BoundNotMet => TIER0_BOUND_NOT_MET,
                    _ => TIER0_INFEASIBLE_OTHER,
                };
                (i.id.index(), code, f64::NAN, Vec::new())
            }));
        }
        let in_window = |id: u32| {
            file.entities
                .binary_search_by_key(&id, |e| e.id.index())
                .ok()
                .is_some_and(|at| file.entities[at].bounds.as_ref().is_none_or(|b| meets(b, &bounds)))
        };
        rows.retain(|r| in_window(r.0));
        rows.sort_unstable_by_key(|r| r.0);
        let wanted =
            file.entities.iter().filter(|e| e.bounds.as_ref().is_none_or(|b| meets(b, &bounds))).count();
        let mut s = Tier0Stream {
            ids: Vec::with_capacity(rows.len()),
            codes: Vec::with_capacity(rows.len()),
            error_bounds: Vec::with_capacity(rows.len()),
            offsets: vec![0],
            vertices: Vec::new(),
            exact: 0,
            previews: 0,
            infeasible: 0,
            uncovered: (wanted - rows.len()) as u32,
            leaves_read,
            leaves_without_block,
            advanced_in_view: u32::from(index.dir.advanced_bounds().is_some_and(|a| a.intersects(&q))),
        };
        for (id, code, error, points) in rows {
            match code {
                TIER0_EXACT => s.exact += 1,
                TIER0_CONTROL_POLYGON | TIER0_CHORD_POLYLINE => s.previews += 1,
                _ => s.infeasible += 1,
            }
            s.ids.push(id);
            s.codes.push(code);
            s.error_bounds.push(error);
            s.vertices.extend(points.iter().flatten());
            s.offsets.push((s.vertices.len() / 2) as u32);
        }
        Ok(self.insert(Item::Tier0(s)))
    }

    /// The Tier 0 stream of a `tier0_viewport` result.
    pub fn tier0(&self, result: u32) -> Result<&Tier0Stream, AbiError> {
        match self.get(result)? {
            Item::Tier0(t) => Ok(t),
            _ => Err(AbiError::WrongKind),
        }
    }

    /// Array `which` of a Tier 0 result: 0 entity ids, 1 codes, 3 offsets (`Uint32Array`), 2 declared
    /// errors and 4 vertices (`Float64Array`).
    pub fn tier0_view(&self, result: u32, which: u32) -> Result<View, AbiError> {
        let t = self.tier0(result)?;
        match which {
            0 => Ok(view_u32(&t.ids)),
            1 => Ok(view_u32(&t.codes)),
            2 => Ok(view_f64(&t.error_bounds)),
            3 => Ok(view_u32(&t.offsets)),
            4 => Ok(view_f64(&t.vertices)),
            _ => Err(AbiError::BadArgument),
        }
    }

    /// Scalar `which` of a Tier 0 result: 0 entries, 1 exact, 2 previews, 3 infeasible, 4 uncovered,
    /// 5 leaves read, 6 leaves without a block, 7 advanced curves in view.
    pub fn tier0_stat(&self, result: u32, which: u32) -> Result<u32, AbiError> {
        let t = self.tier0(result)?;
        match which {
            0 => Ok(t.ids.len() as u32),
            1 => Ok(t.exact),
            2 => Ok(t.previews),
            3 => Ok(t.infeasible),
            4 => Ok(t.uncovered),
            5 => Ok(t.leaves_read),
            6 => Ok(t.leaves_without_block),
            7 => Ok(t.advanced_in_view),
            _ => Err(AbiError::BadArgument),
        }
    }

    /// How many entities the batch holds.
    pub fn batch_len(&self, batch: u32) -> Result<u32, AbiError> {
        match self.get(batch)? {
            Item::Batch { picked, .. } => Ok(picked.len() as u32),
            _ => Err(AbiError::WrongKind),
        }
    }

    /// Whether the budget cut the batch short.
    pub fn batch_truncated(&self, batch: u32) -> Result<bool, AbiError> {
        match self.get(batch)? {
            Item::Batch { truncated, .. } => Ok(*truncated),
            _ => Err(AbiError::WrongKind),
        }
    }

    /// Tessellates the batch into four packed arrays and returns the handle of the result. An entity
    /// whose budget cannot be met gets an empty range of the offset table and is counted by
    /// [`Runtime::failed`]; the batch as a whole still succeeds.
    pub fn tessellate_batch(
        &mut self,
        batch: u32,
        ctx: &TessellationContext,
        precision: Precision,
    ) -> Result<u32, AbiError> {
        let (file, picked) = match self.get(batch)? {
            Item::Batch { file, picked, .. } => (Rc::clone(file), picked),
            _ => return Err(AbiError::WrongKind),
        };
        let mut flat: Vec<f64> = Vec::new();
        let mut offsets = Vec::with_capacity(picked.len() + 1);
        let (mut ids, mut styles) = (Vec::with_capacity(picked.len()), Vec::with_capacity(picked.len()));
        let mut failed = 0;
        offsets.push(0u32);
        for i in picked {
            let e = &file.entities[*i];
            match e.curve.tessellate(ctx) {
                Ok(t) => flat.extend(t.vertices.iter().flatten()),
                Err(PvecError::BadContext { .. }) => return Err(AbiError::BadArgument),
                Err(_) => failed += 1,
            }
            offsets.push((flat.len() / 2) as u32);
            ids.push(e.id.index());
            styles.push(e.style);
        }
        let (vertices, origin) = match precision {
            Precision::F64 => (Vertices::F64(flat), [0.0, 0.0]),
            Precision::F32 => {
                let mut origin = [f64::INFINITY; 2];
                for p in flat.chunks_exact(2) {
                    origin = [origin[0].min(p[0]), origin[1].min(p[1])];
                }
                if flat.is_empty() {
                    origin = [0.0, 0.0];
                }
                let small = flat.iter().enumerate().map(|(i, v)| (v - origin[i % 2]) as f32).collect();
                (Vertices::F32(small), origin)
            }
        };
        Ok(self.insert(Item::Tessellation(Packed { vertices, offsets, ids, styles, origin, failed })))
    }

    /// One view.
    pub fn view(&self, tessellation: u32, kind: BufferKind) -> Result<View, AbiError> {
        let p = self.packed(tessellation)?;
        let at = |s: usize| s as u32;
        Ok(match kind {
            BufferKind::Vertices => match &p.vertices {
                Vertices::F64(v) => {
                    View { ptr: at(v.as_ptr() as usize), len: v.len() as u32, elem: ElemType::F64 }
                }
                Vertices::F32(v) => {
                    View { ptr: at(v.as_ptr() as usize), len: v.len() as u32, elem: ElemType::F32 }
                }
            },
            BufferKind::Offsets => View {
                ptr: at(p.offsets.as_ptr() as usize),
                len: p.offsets.len() as u32,
                elem: ElemType::U32,
            },
            BufferKind::EntityIds => {
                View { ptr: at(p.ids.as_ptr() as usize), len: p.ids.len() as u32, elem: ElemType::U32 }
            }
            BufferKind::StyleRefs => {
                View { ptr: at(p.styles.as_ptr() as usize), len: p.styles.len() as u32, elem: ElemType::U32 }
            }
        })
    }

    /// The four views of a packed tessellation, in [`BufferKind`] order. These four typed arrays are all
    /// the JS objects a result costs, whatever the number of entities.
    pub fn views(&self, tessellation: u32) -> Result<[View; 4], AbiError> {
        Ok([
            self.view(tessellation, BufferKind::Vertices)?,
            self.view(tessellation, BufferKind::Offsets)?,
            self.view(tessellation, BufferKind::EntityIds)?,
            self.view(tessellation, BufferKind::StyleRefs)?,
        ])
    }

    /// The vertex array as `f64`, when the result was made with [`Precision::F64`].
    pub fn vertices_f64(&self, tessellation: u32) -> Result<&[f64], AbiError> {
        match &self.packed(tessellation)?.vertices {
            Vertices::F64(v) => Ok(v),
            Vertices::F32(_) => Err(AbiError::WrongKind),
        }
    }

    /// The vertex array as `f32`, when the result was made with [`Precision::F32`].
    pub fn vertices_f32(&self, tessellation: u32) -> Result<&[f32], AbiError> {
        match &self.packed(tessellation)?.vertices {
            Vertices::F32(v) => Ok(v),
            Vertices::F64(_) => Err(AbiError::WrongKind),
        }
    }

    /// The offset table, `entities + 1` long.
    pub fn offsets(&self, tessellation: u32) -> Result<&[u32], AbiError> {
        Ok(&self.packed(tessellation)?.offsets)
    }

    /// The `EntityId` of each entity.
    pub fn entity_ids(&self, tessellation: u32) -> Result<&[u32], AbiError> {
        Ok(&self.packed(tessellation)?.ids)
    }

    /// The style reference of each entity.
    pub fn style_refs(&self, tessellation: u32) -> Result<&[u32], AbiError> {
        Ok(&self.packed(tessellation)?.styles)
    }

    /// The origin the `f32` vertices are relative to; `[0, 0]` for `f64` vertices.
    pub fn origin(&self, tessellation: u32) -> Result<[f64; 2], AbiError> {
        Ok(self.packed(tessellation)?.origin)
    }

    /// How many entities of the batch got no vertices because their budget could not be met.
    pub fn failed(&self, tessellation: u32) -> Result<u32, AbiError> {
        Ok(self.packed(tessellation)?.failed)
    }

    /// The conformance tag of the result: always [`DISPLAY_FIDELITY`]. A result cannot be made without it.
    pub fn conformance(&self, tessellation: u32) -> Result<u32, AbiError> {
        self.packed(tessellation).map(|_| DISPLAY_FIDELITY)
    }

    /// The memory epoch: it changes whenever linear memory has grown. On `wasm32` it is the size of
    /// memory 0 in pages, which only ever grows; elsewhere it counts [`Runtime::force_grow`] calls.
    pub fn memory_epoch(&self) -> u32 {
        #[cfg(target_arch = "wasm32")]
        {
            core::arch::wasm32::memory_size::<0>() as u32
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.grown
        }
    }

    /// Grows linear memory by one page, as `memory.grow` would. On `wasm32` it does; elsewhere it
    /// advances the epoch the same way.
    pub fn force_grow(&mut self) {
        #[cfg(target_arch = "wasm32")]
        {
            core::arch::wasm32::memory_grow::<0>(1);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.grown += 1;
        }
    }

    /// Whether a view read at `epoch` from `handle` can still be used: the handle is live and linear
    /// memory has not grown since. `Err(StaleView)` means make the typed arrays again.
    pub fn view_current(&self, handle: u32, epoch: u32) -> Result<(), AbiError> {
        self.get(handle)?;
        if epoch == self.memory_epoch() { Ok(()) } else { Err(AbiError::StaleView) }
    }

    /// Releases a handle of any kind. A second release, and any use afterwards, is an error.
    pub fn release(&mut self, handle: u32) -> Result<(), AbiError> {
        self.get(handle)?;
        let slot = &mut self.slots[(handle & ((1 << INDEX_BITS) - 1)) as usize];
        slot.item = None;
        slot.generation = slot.generation.wrapping_add(1);
        Ok(())
    }
}

fn view_u32(v: &[u32]) -> View {
    View { ptr: v.as_ptr() as usize as u32, len: v.len() as u32, elem: ElemType::U32 }
}

fn view_f64(v: &[f64]) -> View {
    View { ptr: v.as_ptr() as usize as u32, len: v.len() as u32, elem: ElemType::F64 }
}
