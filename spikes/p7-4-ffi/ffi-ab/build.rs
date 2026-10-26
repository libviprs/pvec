//! Candidate (b): the header is generated from the Rust source on every build, so it cannot drift from it.

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets the manifest dir");
    let config = cbindgen::Config {
        language: cbindgen::Language::C,
        include_guard: Some("PVEC_B_H".to_string()),
        enumeration: cbindgen::EnumConfig {
            prefix_with_name: true,
            rename_variants: cbindgen::RenameRule::ScreamingSnakeCase,
            ..Default::default()
        },
        ..Default::default()
    };
    // Not into the source tree: a header that changes the tree on every build is a header nobody reviews.
    println!("cargo:rerun-if-env-changed=PVEC_B_HEADER_DIR");
    let out = std::env::var_os("PVEC_B_HEADER_DIR")
        .or_else(|| std::env::var_os("OUT_DIR"))
        .map(std::path::PathBuf::from)
        .expect("cargo sets OUT_DIR");
    std::fs::create_dir_all(&out).expect("create the header directory");
    cbindgen::Builder::new()
        .with_crate(&dir)
        .with_config(config)
        .generate()
        .expect("cbindgen generates the header")
        .write_to_file(out.join("pvec_b.h"));
}
