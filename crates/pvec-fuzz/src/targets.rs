//! The six fuzz targets.

/// A target: bytes in, the word for what it reached out.
pub type Target = fn(&[u8]) -> String;

/// The targets by name.
pub const TARGETS: [(&str, Target); 6] = [
    ("fuzz_header", fuzz_header),
    ("fuzz_directory_traversal", fuzz_directory_traversal),
    ("fuzz_immediate_blocks", fuzz_immediate_blocks),
    ("fuzz_advanced_blocks", fuzz_advanced_blocks),
    ("fuzz_decompress_boundary", fuzz_decompress_boundary),
    ("fuzz_component_streams", fuzz_component_streams),
];

/// The header and the manifest.
pub fn fuzz_header(_data: &[u8]) -> String {
    todo!("P7.8")
}

/// A whole artifact: open, plan, read every block, validate.
pub fn fuzz_directory_traversal(_data: &[u8]) -> String {
    todo!("P7.8")
}

/// One hot (immediate geometry) block.
pub fn fuzz_immediate_blocks(_data: &[u8]) -> String {
    todo!("P7.8")
}

/// The six advanced blocks (records, knots, weights, control points, segments, trims), framed as six `u32`
/// lengths and then the blocks.
pub fn fuzz_advanced_blocks(_data: &[u8]) -> String {
    todo!("P7.8")
}

/// What a decompress boundary does with the stream: see [`decompress_boundary`].
pub fn fuzz_decompress_boundary(_data: &[u8]) -> String {
    todo!("P7.8")
}

/// A component stream set.
pub fn fuzz_component_streams(_data: &[u8]) -> String {
    todo!("P7.8")
}

/// What the reader did with a block whose codec, declared length and stored bytes the input chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// The input is shorter than its frame.
    NotFramed,
    /// The reader refused, naming the rule.
    Refused(String),
    /// The block decoded to this many bytes.
    Decoded(usize),
}

/// Frames the input as `[codec: u8][declared length: u32 le][stored bytes]`, lays it out as a one-block
/// artifact and reads the block back through the reader. A declared length beyond the expansion ratio of the
/// stored length is a refusal and never a decode.
pub fn decompress_boundary(_data: &[u8]) -> Boundary {
    todo!("P7.8")
}

/// The input with its block checksums and artifact digest recomputed, when it still lays out as a container.
pub fn reseal(_data: &[u8]) -> Option<Vec<u8>> {
    todo!("P7.8")
}
