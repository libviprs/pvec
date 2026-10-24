//! Issue 121: `Trimmed::from_bulge` hands the writer numbers that must not depend on the C library, so
//! its body may not call the platform `atan` or `atan2`.

use std::fs;
use std::path::PathBuf;

#[test]
fn from_bulge_does_not_call_the_platform_arctangent() {
    let text = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/curve.rs")).unwrap();
    let from = text.find("pub fn from_bulge").expect("from_bulge exists");
    let body = &text[from..];
    let end = body.find("pub fn from_normalised").expect("from_normalised follows");
    let body = &body[..end];
    assert!(!body.contains(".atan2("), "from_bulge calls the platform atan2");
    assert!(!body.contains(".atan()"), "from_bulge calls the platform atan");
    assert!(body.contains("detmath::"), "from_bulge takes its angles from pvec_numeric::detmath");
}
