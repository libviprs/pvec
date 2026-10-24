//! Test support for the container: the canonical artifact generator (issue P2.4) and the reference
//! reader that the hostile-container tests hold a reader to.
//!
//! Dependency direction is `pvec-gold-fixtures` -> `pvec`: this crate reads the vendored bundle
//! through `pvec-corpus` and writes through the real `pvec-container` writer. It has no dependency
//! on `libviprs`, on a CAD reader, or on a PDF reader.

pub mod generate;
pub mod reference;
