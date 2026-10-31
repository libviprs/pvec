//! Test support for the container: the canonical artifact generator (issue P2.4) and the reference
//! reader that the hostile-container tests hold a reader to.
//!
//! Dependency direction is `pvec-gold-fixtures` -> `pvec`: this crate reads the vendored bundle
//! through `pvec-corpus` and writes through the real `pvec-container` writer. It has no dependency
//! on `libviprs`, on a CAD reader, or on a PDF reader.

pub use pvec_numeric::detmath;
pub mod generate;
pub mod hostile;
pub mod raster_hostile;
pub mod records;
pub mod reference;
pub mod tilegen;
pub mod tiler;
