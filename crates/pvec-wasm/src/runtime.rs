use crate::abi::{
    TIER0_BOUND_NOT_MET, TIER0_CHORD_POLYLINE, TIER0_CONTROL_POLYGON, TIER0_EVALUATOR_REFUSED, TIER0_EXACT,
    TIER0_INFEASIBLE_OTHER, TIER0_VERTEX_CAP,
};
use crate::{AbiError, BufferKind, DISPLAY_FIDELITY, ElemType, Precision, View};
use pvec::unstable::{Aabb, InfeasibleReason, PreviewForm, SpatialDirectory, Tier0Leaves};
use pvec::{Bounds2, Curve, CurveEval, EntityId, PvecError, PvecReader, Tessellate, TessellationContext};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// One entity of an open file.
struct Entity {
    id: EntityId,
    curve: Curve,
    /// `None` for a curve that has no bounds (an unbounded line), which no viewport can cull.
    bounds: Option<Bounds2>,
    style: u32,
    /// The leaf whose hot block it was read from, `None` for an advanced curve.
    cell: Option<u32>,
}

/// The spatial directory and the Tier 0 leaf table of a file that has them.
struct Index {
    dir: SpatialDirectory,
    leaves: Tier0Leaves,
}

/// The bytes a host has provided for a range-opened file, and the ranges the last call that could not
/// finish still needs. The reader's read callback and the runtime share it, so it is behind a lock
/// (contract: the callback has to be `Send + Sync`).
struct Ranges {
    len: u64,
    state: Mutex<RangeState>,
}

#[derive(Default)]
struct RangeState {
    /// Disjoint, ascending, never adjacent: provided ranges are merged.
    have: Vec<(u64, Vec<u8>)>,
    needed: Vec<(u64, u32)>,
}

impl Ranges {
    fn new(len: u64) -> Arc<Ranges> {
        Arc::new(Ranges { len, state: Mutex::new(RangeState::default()) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RangeState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keeps `bytes` as the content of `offset..`, merging with whatever it overlaps or touches.
    fn provide(&self, offset: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let end = offset + bytes.len() as u64;
        let mut st = self.lock();
        let (mut start, mut stop) = (offset, end);
        let mut merged: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut absorbed: Vec<(u64, Vec<u8>)> = Vec::new();
        for (o, b) in st.have.drain(..) {
            if o <= end && o + b.len() as u64 >= offset {
                start = start.min(o);
                stop = stop.max(o + b.len() as u64);
                absorbed.push((o, b));
            } else {
                merged.push((o, b));
            }
        }
        let mut whole = vec![0u8; (stop - start) as usize];
        // older bytes first, so that what was just provided wins where they overlap
        for (o, b) in &absorbed {
            whole[(o - start) as usize..(o - start) as usize + b.len()].copy_from_slice(b);
        }
        whole[(offset - start) as usize..(offset - start) as usize + bytes.len()].copy_from_slice(bytes);
        merged.push((start, whole));
        merged.sort_by_key(|(o, _)| *o);
        st.have = merged;
    }

    fn covers(have: &[(u64, Vec<u8>)], offset: u64, len: u32) -> bool {
        have.iter().any(|(o, b)| *o <= offset && offset + u64::from(len) <= *o + b.len() as u64)
    }

    /// The bytes of `offset..offset + len` when they were provided. When they were not, the range is
    /// noted as needed and the read fails, which is how the reader finds out it has to stop.
    fn read(&self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        let mut st = self.lock();
        if let Some((o, b)) =
            st.have.iter().find(|(o, b)| *o <= offset && offset + u64::from(len) <= *o + b.len() as u64)
        {
            let at = (offset - o) as usize;
            return Ok(b[at..at + len as usize].to_vec());
        }
        if !st.needed.contains(&(offset, len)) {
            st.needed.push((offset, len));
        }
        Err("range not provided".to_string())
    }

    fn clear_needed(&self) {
        self.lock().needed.clear();
    }

    fn needed(&self) -> Vec<(u64, u32)> {
        self.lock().needed.clone()
    }

    /// Notes the part of `wanted` that was not provided as needed. True when something is.
    fn need_all(&self, wanted: Vec<(u64, u32)>) -> bool {
        let mut st = self.lock();
        let missing: Vec<(u64, u32)> =
            wanted.into_iter().filter(|(o, l)| !Ranges::covers(&st.have, *o, *l)).collect();
        st.needed = missing;
        !st.needed.is_empty()
    }
}

/// What a range-opened file has read so far: which leaves' hot blocks, and whether the advanced curves, are
/// in `File::entities`.
#[derive(Default)]
struct Loaded {
    cells: HashSet<u32>,
    advanced: bool,
    everything: bool,
}

/// What a file opened by ranges keeps besides its reader.
struct Ranged {
    ranges: Arc<Ranges>,
    loaded: RefCell<Loaded>,
}

/// What an open file keeps: the entities, keyed by id, and its index. A file opened from whole bytes has
/// every entity from the start; one opened by ranges has the entities of the leaves its calls have read.
/// A batch shares the entities it holds, so releasing the file does not pull the curves out from under it.
/// A directory the reader refused is kept as the error, so that only the calls that need it fail. The
/// reader is kept when a call can still read through it: the file has an index, or it is behind ranges.
struct File {
    reader: Option<PvecReader>,
    entities: RefCell<BTreeMap<u32, Rc<Entity>>>,
    index: Result<Option<Index>, AbiError>,
    ranged: Option<Ranged>,
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

fn entity(cell: Option<u32>, id: EntityId, curve: Curve, style: u32) -> (u32, Rc<Entity>) {
    (id.index(), Rc::new(Entity { id, bounds: curve.bounds().ok(), curve, style, cell }))
}

/// The spatial directory and Tier 0 leaf table of an opened reader, `Ok(None)` for a file with no directory.
fn index_of(reader: &PvecReader) -> Result<Option<Index>, AbiError> {
    match reader.spatial_directory() {
        Ok(Some(dir)) => match reader.tier0_leaves(&dir) {
            Ok(leaves) => Ok(Some(Index { dir, leaves })),
            Err(_) => Err(AbiError::Read),
        },
        Ok(None) => Ok(None),
        Err(_) => Err(AbiError::Read),
    }
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
    Batch {
        picked: Vec<Rc<Entity>>,
        truncated: bool,
    },
    /// A file opened by ranges that has not finished opening: it holds the ranges provided so far.
    Opening(Arc<Ranges>),
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
/// A handle is a 15 bit generation and a 16 bit index in one `u32` below 2^31, so a handle that was
/// released stays refused for the next 32,767 reuses of its slot. Nothing here is shared between threads: a module instance is one thread.
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
        let read = |_: PvecError| AbiError::Read;
        let reader = PvecReader::open(bytes).map_err(read)?;
        let loaded = reader.entities_in(None, true).map_err(read)?;
        let entities =
            loaded.into_iter().map(|(cell, id, curve, style)| entity(cell, id, curve, style)).collect();
        let index = index_of(&reader);
        let reader = if matches!(index, Ok(None)) { None } else { Some(reader) };
        Ok(self.insert(Item::File(Rc::new(File {
            reader,
            entities: RefCell::new(entities),
            index,
            ranged: None,
        }))))
    }

    /// Starts opening a file of `len` bytes that the host serves by byte ranges, and returns a handle that
    /// becomes the file's when [`Runtime::open_poll`] succeeds. Nothing is read yet.
    pub fn open_ranged(&mut self, len: u64) -> Result<u32, AbiError> {
        if len == 0 {
            return Err(AbiError::BadArgument);
        }
        Ok(self.insert(Item::Opening(Ranges::new(len))))
    }

    fn ranges_of(&self, handle: u32) -> Result<Arc<Ranges>, AbiError> {
        match self.get(handle)? {
            Item::Opening(r) => Ok(Arc::clone(r)),
            Item::File(f) => f.ranged.as_ref().map(|r| Arc::clone(&r.ranges)).ok_or(AbiError::WrongKind),
            _ => Err(AbiError::WrongKind),
        }
    }

    /// Hands the module the bytes at `offset` of a range-opened file. It may be more than was asked for,
    /// and a later call is served from what was provided without asking again.
    pub fn provide_range(&mut self, handle: u32, offset: u64, bytes: &[u8]) -> Result<(), AbiError> {
        let ranges = self.ranges_of(handle)?;
        match offset.checked_add(bytes.len() as u64) {
            Some(end) if end <= ranges.len => {
                ranges.provide(offset, bytes);
                Ok(())
            }
            _ => Err(AbiError::BadArgument),
        }
    }

    /// The ranges the last call on `handle` that returned [`AbiError::NeedRange`] needs, in the order the
    /// reader would ask for them. The list is empty after a call that finished.
    pub fn needed(&self, handle: u32) -> Result<Vec<(u64, u32)>, AbiError> {
        Ok(self.ranges_of(handle)?.needed())
    }

    /// Tries to finish opening: it reads the header, the manifest, the block directory and the spatial
    /// directory from what was provided, and no entity. It returns the handle (now a file) or
    /// [`AbiError::NeedRange`], and may be called again after the ranges in [`Runtime::needed`] are provided.
    pub fn open_poll(&mut self, handle: u32) -> Result<u32, AbiError> {
        let ranges = match self.get(handle)? {
            Item::Opening(r) => Arc::clone(r),
            Item::File(f) if f.ranged.is_some() => return Ok(handle),
            _ => return Err(AbiError::WrongKind),
        };
        ranges.clear_needed();
        let source = Arc::clone(&ranges);
        let opened = PvecReader::open_ranges(ranges.len, move |offset, len| source.read(offset, len));
        let reader = match opened {
            Ok(r) => r,
            Err(_) => {
                return Err(if ranges.needed().is_empty() { AbiError::Read } else { AbiError::NeedRange });
            }
        };
        let index = index_of(&reader);
        if index.is_err() && !ranges.needed().is_empty() {
            return Err(AbiError::NeedRange);
        }
        let file = File {
            reader: Some(reader),
            entities: RefCell::new(BTreeMap::new()),
            index,
            ranged: Some(Ranged { ranges, loaded: RefCell::new(Loaded::default()) }),
        };
        let at = (handle & ((1 << INDEX_BITS) - 1)) as usize;
        self.slots[at].item = Some(Item::File(Rc::new(file)));
        Ok(handle)
    }

    /// Reads the entities a window needs into a range-opened file: the hot blocks of the leaves it touches
    /// and, when their bounds can meet it, the advanced curves. Every range that is missing is named at
    /// once ([`AbiError::NeedRange`]). A file opened from whole bytes has all of them already.
    fn load(&self, file: &File, window: &Bounds2) -> Result<(), AbiError> {
        let Some(ranged) = &file.ranged else { return Ok(()) };
        let reader = file.reader.as_ref().ok_or(AbiError::Read)?;
        let (cells, advanced) = match file.index.as_ref().map_err(|e| *e)? {
            Some(index) => {
                let q = Aabb::new(window.min, window.max).map_err(|_| AbiError::BadArgument)?;
                let cells: Vec<u32> = index.dir.query(&q).map(|c| c.id.get()).collect();
                (Some(cells), index.dir.advanced_bounds().is_none_or(|a| a.intersects(&q)))
            }
            None => (None, true),
        };
        self.load_cells(file, ranged, reader, cells.as_deref(), advanced)
    }

    fn load_cells(
        &self,
        file: &File,
        ranged: &Ranged,
        reader: &PvecReader,
        cells: Option<&[u32]>,
        advanced: bool,
    ) -> Result<(), AbiError> {
        let (want_cells, want_advanced) = {
            let have = ranged.loaded.borrow();
            if have.everything {
                return Ok(());
            }
            let missing =
                cells.map(|c| c.iter().copied().filter(|c| !have.cells.contains(c)).collect::<Vec<_>>());
            (missing, advanced && !have.advanced)
        };
        if want_cells.as_ref().is_some_and(Vec::is_empty) && !want_advanced {
            return Ok(());
        }
        ranged.ranges.clear_needed();
        let wanted =
            reader.entity_ranges(want_cells.as_deref(), want_advanced).map_err(|_| AbiError::Read)?;
        if ranged.ranges.need_all(wanted) {
            return Err(AbiError::NeedRange);
        }
        let read = reader.entities_in(want_cells.as_deref(), want_advanced);
        let read = read.map_err(|_| {
            if ranged.ranges.needed().is_empty() { AbiError::Read } else { AbiError::NeedRange }
        })?;
        let mut entities = file.entities.borrow_mut();
        for (cell, id, curve, style) in read {
            let (key, e) = entity(cell, id, curve, style);
            entities.insert(key, e);
        }
        let mut have = ranged.loaded.borrow_mut();
        match want_cells {
            Some(c) => have.cells.extend(c),
            None => have.everything = true,
        }
        have.advanced |= want_advanced;
        Ok(())
    }

    /// The entities of `file` whose bounds meet `bounds`, at most `budget` of them in ascending id
    /// order, as a batch handle. On a file opened by ranges it reads the leaves the window touches first,
    /// and returns [`AbiError::NeedRange`] until the host has provided them.
    pub fn query_viewport(&mut self, file: u32, bounds: Bounds2, budget: u32) -> Result<u32, AbiError> {
        let file = self.file(file)?;
        if bounds.min.iter().chain(&bounds.max).any(|v| v.is_nan()) {
            return Err(AbiError::BadArgument);
        }
        self.load(&file, &bounds)?;
        let mut picked: Vec<Rc<Entity>> = file
            .entities
            .borrow()
            .values()
            .filter(|e| e.bounds.as_ref().is_none_or(|b| meets(b, &bounds)))
            .cloned()
            .collect();
        let truncated = picked.len() > budget as usize;
        picked.truncate(budget as usize);
        Ok(self.insert(Item::Batch { picked, truncated }))
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
        let reader = file.reader.as_ref().ok_or(AbiError::Read)?;
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
            list.stored_bytes.push(reader.stored_bytes(cell.blocks) as f64);
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
        let reader = file.reader.as_ref().ok_or(AbiError::Read)?;
        let q = Aabb::new(bounds.min, bounds.max).map_err(|_| AbiError::BadArgument)?;
        let touched: Vec<_> = index.dir.query(&q).collect();
        let inside = |c: &pvec::unstable::Cell| {
            c.bounds.min()[0] >= q.min()[0]
                && c.bounds.min()[1] >= q.min()[1]
                && c.bounds.max()[0] <= q.max()[0]
                && c.bounds.max()[1] <= q.max()[1]
        };
        if let Some(ranged) = &file.ranged {
            // The Tier 0 blocks of the leaves the window touches, and the hot blocks of the leaves it only
            // cuts: a leaf inside the window has all its entities in it, so its count is known without them.
            let with_block: Vec<_> =
                touched.iter().map(|c| c.id).filter(|id| index.leaves.block(*id).is_some()).collect();
            let cut: Vec<u32> = touched.iter().filter(|c| !inside(c)).map(|c| c.id.get()).collect();
            let advanced = index.dir.advanced_bounds().is_none_or(|a| a.intersects(&q));
            ranged.ranges.clear_needed();
            let mut wanted = reader.tier0_ranges(&index.leaves, &with_block).map_err(|_| AbiError::Read)?;
            let have = ranged.loaded.borrow();
            let cut_missing: Vec<u32> =
                cut.iter().copied().filter(|c| !have.cells.contains(c) && !have.everything).collect();
            let adv_missing = advanced && !have.advanced && !have.everything;
            drop(have);
            if !cut_missing.is_empty() || adv_missing {
                wanted.extend(
                    reader.entity_ranges(Some(&cut_missing), adv_missing).map_err(|_| AbiError::Read)?,
                );
            }
            if ranged.ranges.need_all(wanted) {
                return Err(AbiError::NeedRange);
            }
            self.load_cells(&file, ranged, reader, Some(&cut), advanced)?;
        }
        let ranged_counts = file.ranged.as_ref().map(|_| {
            touched
                .iter()
                .filter(|c| inside(c))
                .map(|c| (c.id.get(), c.entities))
                .collect::<BTreeMap<u32, u32>>()
        });
        let mut rows: Vec<(u32, u32, f64, Vec<[f64; 2]>)> = Vec::new();
        let (mut leaves_read, mut leaves_without_block) = (0, 0);
        for cell in index.dir.query(&q) {
            let plan = reader.tier0_plan(&index.leaves, cell.id).map_err(|_| AbiError::Read)?;
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
        let entities = file.entities.borrow();
        // A leaf inside the window is not loaded on a file opened by ranges, and all of its entities are in the window.
        let in_window = |id: u32| match entities.get(&id) {
            Some(e) => e.bounds.as_ref().is_none_or(|b| meets(b, &bounds)),
            None => ranged_counts.is_some(),
        };
        rows.retain(|r| in_window(r.0));
        rows.sort_unstable_by_key(|r| r.0);
        let wanted = match &ranged_counts {
            None => entities.values().filter(|e| e.bounds.as_ref().is_none_or(|b| meets(b, &bounds))).count(),
            Some(inside) => {
                let whole: usize = inside.values().map(|n| *n as usize).sum();
                let cut = entities
                    .values()
                    .filter(|e| e.cell.is_none_or(|c| !inside.contains_key(&c)))
                    .filter(|e| e.bounds.as_ref().is_none_or(|b| meets(b, &bounds)))
                    .count();
                whole + cut
            }
        };
        let mut s = Tier0Stream {
            ids: Vec::with_capacity(rows.len()),
            codes: Vec::with_capacity(rows.len()),
            error_bounds: Vec::with_capacity(rows.len()),
            offsets: vec![0],
            vertices: Vec::new(),
            exact: 0,
            previews: 0,
            infeasible: 0,
            uncovered: wanted.saturating_sub(rows.len()) as u32,
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
        let picked = match self.get(batch)? {
            Item::Batch { picked, .. } => picked,
            _ => return Err(AbiError::WrongKind),
        };
        let mut flat: Vec<f64> = Vec::new();
        let mut offsets = Vec::with_capacity(picked.len() + 1);
        let (mut ids, mut styles) = (Vec::with_capacity(picked.len()), Vec::with_capacity(picked.len()));
        let mut failed = 0;
        offsets.push(0u32);
        for e in picked {
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
        // 15 bits, so a handle always fits the non-negative `i32` the exports return (#203).
        slot.generation = (slot.generation + 1) & 0x7fff;
        Ok(())
    }
}

fn view_u32(v: &[u32]) -> View {
    View { ptr: v.as_ptr() as usize as u32, len: v.len() as u32, elem: ElemType::U32 }
}

fn view_f64(v: &[f64]) -> View {
    View { ptr: v.as_ptr() as usize as u32, len: v.len() as u32, elem: ElemType::F64 }
}
