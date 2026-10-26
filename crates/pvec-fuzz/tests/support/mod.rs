//! Shared by the tests: a counting allocator and a few helpers.
#![allow(dead_code, unsafe_code)]

#[path = "../../../pvec-testkit/tests/support/counting_alloc.rs"]
pub mod counting_alloc;

use pvec_container::reader::{ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::MemorySource;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Bytes the closure requested on this thread.
pub fn metered(f: &mut dyn FnMut()) -> usize {
    let before = counting_alloc::requested();
    f();
    counting_alloc::requested() - before
}

/// Whether the reader opens and fully validates `bytes`, or the rule that refused.
pub fn reads(bytes: &[u8]) -> Result<(), String> {
    let source = MemorySource::new(bytes.to_vec());
    let r: Result<(), ReadError> = (|| {
        let reader = ContainerReader::open(source, ReaderConfig::default())?;
        reader.validate()?;
        reader.verify_artifact_digest()?;
        Ok(())
    })();
    r.map_err(|e| e.rule())
}

/// Runs `f`, returning the panic message if it panicked.
pub fn panics(f: impl FnOnce()) -> Option<String> {
    catch_unwind(AssertUnwindSafe(f)).err().map(|e| {
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "a panic".into())
    })
}

pub fn repo() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
