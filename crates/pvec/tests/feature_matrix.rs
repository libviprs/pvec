//! Issue #52: the same semantic type list under every feature combination of the matrix.

#[allow(unused_imports)]
use pvec::{
    Bounds2, CapabilityId, Curve, CurveRef, Domain, EntityId, Manifest, PvecError, PvecReader, PvecWriter,
    Transform2,
};
#[allow(unused_imports)]
use pvec::components::{
    Fill, HatchStream, LayerStream, LineStyleStream, ProvenanceStream, SemanticClassStream, Skipped, Stream,
    TextStream,
};

#[test]
fn issue_52_the_frozen_core_resolves_under_this_feature_combination() {
    // The imports above are the assertion: they compile under std, alloc, mmap, http, wasm and
    // unstable-stream alike, and a missing or renamed type fails the build.
    assert_eq!(std::mem::size_of::<EntityId>(), 4);
}

#[cfg(feature = "unstable-stream")]
#[test]
fn issue_52_the_unstable_stream_types_resolve_with_the_feature() {
    #[allow(unused_imports)]
    use pvec::unstable::{CancelToken, Generation, GenerationClock, ReadPlan, Scene, Tier0Plan, ViewportQuery};
}
