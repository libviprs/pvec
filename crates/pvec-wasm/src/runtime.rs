use crate::{AbiError, BufferKind, DISPLAY_FIDELITY, ElemType, Precision, View};
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

/// What an open file keeps: the entities, in ascending id order. A batch shares it, so releasing the
/// file does not pull the curves out from under a batch.
struct File {
    entities: Vec<Entity>,
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
        Ok(self.insert(Item::File(Rc::new(File { entities }))))
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
        let meets = |b: &Bounds2| {
            b.min[0] <= bounds.max[0]
                && b.max[0] >= bounds.min[0]
                && b.min[1] <= bounds.max[1]
                && b.max[1] >= bounds.min[1]
        };
        let mut picked: Vec<usize> = (0..file.entities.len())
            .filter(|i| file.entities[*i].bounds.as_ref().is_none_or(meets))
            .collect();
        let truncated = picked.len() > budget as usize;
        picked.truncate(budget as usize);
        Ok(self.insert(Item::Batch { file, picked, truncated }))
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
