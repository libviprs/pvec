/// The version of the packed layouts and the export set. It changes under the rules in
/// `docs/WASM_ABI.md` and in P7.6.
pub const WASM_ABI_VERSION: u32 = 1;

/// The value of the conformance tag a packed tessellation carries: the display approximation of
/// `pvec::Conformance::DisplayFidelity`, the only value there is.
pub const DISPLAY_FIDELITY: u32 = 1;

/// Why a call was refused. The exports return the negative [`AbiError::code`] instead, so that a failure
/// allocates no JS object either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbiError {
    /// The handle was never issued, or has been released.
    Released,
    /// The linear memory has grown since the view was read: typed arrays over the old buffer are
    /// detached and must be made again.
    StaleView,
    /// An argument is out of range or not a number.
    BadArgument,
    /// The bytes are not a PVec file this reader accepts.
    Read,
    /// The handle is live but is not the kind this call takes.
    WrongKind,
}

impl AbiError {
    /// The code an export returns: negative, one per variant, fixed for `WASM_ABI_VERSION` 1.
    pub fn code(self) -> i32 {
        match self {
            AbiError::Released => -1,
            AbiError::StaleView => -2,
            AbiError::BadArgument => -3,
            AbiError::Read => -4,
            AbiError::WrongKind => -5,
        }
    }
}

/// The element type of a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElemType {
    /// `Float64Array`.
    F64,
    /// `Float32Array`.
    F32,
    /// `Uint32Array`.
    U32,
}

/// The precision of the vertex array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// `Float64Array`, source coordinates as they are.
    F64,
    /// `Float32Array`, source coordinates minus the batch origin, for a GPU.
    F32,
}

/// The four arrays of a packed tessellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    /// Interleaved `x, y` vertices.
    Vertices,
    /// `entities + 1` vertex indices; entity `i` owns vertices `offsets[i]..offsets[i + 1]`.
    Offsets,
    /// One `EntityId` per entity.
    EntityIds,
    /// One style reference per entity.
    StyleRefs,
}

/// Where an array is in linear memory, and what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    /// Byte offset in linear memory.
    pub ptr: u32,
    /// Number of elements.
    pub len: u32,
    /// Element type.
    pub elem: ElemType,
}
