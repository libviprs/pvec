use crate::{AbiError, BufferKind, Precision, View};
use pvec::{Bounds2, TessellationContext};

/// Every handle, every batch and every packed result of one module instance.
#[derive(Debug, Default)]
pub struct Runtime {}

impl Runtime {
    /// A runtime with nothing open.
    pub fn new() -> Runtime {
        Runtime {}
    }

    /// Opens a PVec file held in memory and returns its handle.
    pub fn open(&mut self, _bytes: Vec<u8>) -> Result<u32, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The entities of `file` whose bounds meet `bounds`, at most `budget` of them, as a batch handle.
    pub fn query_viewport(&mut self, _file: u32, _bounds: Bounds2, _budget: u32) -> Result<u32, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// How many entities the batch holds.
    pub fn batch_len(&self, _batch: u32) -> Result<u32, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// Whether the budget cut the batch short.
    pub fn batch_truncated(&self, _batch: u32) -> Result<bool, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// Tessellates the batch into four packed arrays and returns the handle of the result.
    pub fn tessellate_batch(
        &mut self,
        _batch: u32,
        _ctx: &TessellationContext,
        _precision: Precision,
    ) -> Result<u32, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The four views of a packed tessellation, in [`BufferKind`] order.
    pub fn views(&self, _tessellation: u32) -> Result<[View; 4], AbiError> {
        Err(AbiError::BadArgument)
    }

    /// One view.
    pub fn view(&self, _tessellation: u32, _kind: BufferKind) -> Result<View, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The vertex array as `f64`, when the result was made with [`Precision::F64`].
    pub fn vertices_f64(&self, _tessellation: u32) -> Result<&[f64], AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The vertex array as `f32`, when the result was made with [`Precision::F32`].
    pub fn vertices_f32(&self, _tessellation: u32) -> Result<&[f32], AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The offset table, `entities + 1` long.
    pub fn offsets(&self, _tessellation: u32) -> Result<&[u32], AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The `EntityId` of each entity.
    pub fn entity_ids(&self, _tessellation: u32) -> Result<&[u32], AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The style reference of each entity.
    pub fn style_refs(&self, _tessellation: u32) -> Result<&[u32], AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The origin the `f32` vertices are relative to; `[0, 0]` for `f64` vertices.
    pub fn origin(&self, _tessellation: u32) -> Result<[f64; 2], AbiError> {
        Err(AbiError::BadArgument)
    }

    /// How many entities of the batch got no vertices because their budget could not be met.
    pub fn failed(&self, _tessellation: u32) -> Result<u32, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The conformance tag of the result: always [`crate::DISPLAY_FIDELITY`].
    pub fn conformance(&self, _tessellation: u32) -> Result<u32, AbiError> {
        Err(AbiError::BadArgument)
    }

    /// The memory epoch: it changes whenever linear memory has grown.
    pub fn memory_epoch(&self) -> u32 {
        0
    }

    /// Grows linear memory by one page, as `memory.grow` would. On `wasm32` it does; elsewhere it
    /// advances the epoch the same way.
    pub fn force_grow(&mut self) {}

    /// Whether a view read at `epoch` from `handle` can still be used.
    pub fn view_current(&self, _handle: u32, _epoch: u32) -> Result<(), AbiError> {
        Err(AbiError::BadArgument)
    }

    /// Releases a handle of any kind. A second release, and any use afterwards, is an error.
    pub fn release(&mut self, _handle: u32) -> Result<(), AbiError> {
        Err(AbiError::BadArgument)
    }
}
