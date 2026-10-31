/// The version of the packed layouts and the export set. It changes under the rules in
/// `docs/WASM_ABI.md` and in P7.6.
pub const WASM_ABI_VERSION: u32 = 2;

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
    /// The file has no spatial directory, which the call needs (version 2).
    NoIndex,
    /// The call needs byte ranges of a range-opened file that the host has not provided yet. The ranges
    /// are in [`crate::Runtime::needed`]; provide them and call again.
    NeedRange,
    /// The file has no raster stream (issue #221, additive).
    NoRaster,
    /// The tile is not stored: the index says length 0, and it draws nothing (issue #221, additive).
    TileNotStored,
    /// The raster is skipped by a `RASTER-NN` rule, which `raster_skipped` names (issue #221, additive).
    RasterSkipped,
}

impl AbiError {
    /// The code an export returns: negative, one per variant, fixed since `WASM_ABI_VERSION` 1; version 2 adds -6, issue #205 adds -7 and issue #221 adds -8 to -10.
    pub fn code(self) -> i32 {
        match self {
            AbiError::Released => -1,
            AbiError::StaleView => -2,
            AbiError::BadArgument => -3,
            AbiError::Read => -4,
            AbiError::WrongKind => -5,
            AbiError::NoIndex => -6,
            AbiError::NeedRange => -7,
            AbiError::NoRaster => -8,
            AbiError::TileNotStored => -9,
            AbiError::RasterSkipped => -10,
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
    /// `Uint8Array` (issue #221: the encoded bytes of a raster tile and the text of a placement).
    U8,
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

/// Tier 0 code of an entity that goes out exact: its hot block carries the geometry.
pub const TIER0_EXACT: u32 = 0;
/// Tier 0 code of a preview that is the control polygon of the curve.
pub const TIER0_CONTROL_POLYGON: u32 = 1;
/// Tier 0 code of a preview that is a polyline whose chords stay within the declared error.
pub const TIER0_CHORD_POLYLINE: u32 = 2;
/// Tier 0 code of an entity declared infeasible because the exact evaluator refused its curve.
pub const TIER0_EVALUATOR_REFUSED: u32 = 3;
/// Tier 0 code of an entity declared infeasible because no preview fits in the vertex cap.
pub const TIER0_VERTEX_CAP: u32 = 4;
/// Tier 0 code of an entity declared infeasible because its preview misses the bound.
pub const TIER0_BOUND_NOT_MET: u32 = 5;
/// Tier 0 code of an entity declared infeasible for a reason this version does not know.
pub const TIER0_INFEASIBLE_OTHER: u32 = 6;
