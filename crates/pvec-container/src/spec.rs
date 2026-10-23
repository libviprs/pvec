//! The field tables behind `docs/CONTAINER.md` chapter 1.

/// One field of a fixed structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// Field name.
    pub name: &'static str,
    /// Byte offset inside the structure.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
    /// Encoding, for the document.
    pub encoding: &'static str,
    /// One-line meaning.
    pub meaning: &'static str,
}

/// The header fields.
pub const HEADER_FIELDS: &[FieldSpec] = &[];
/// The fixed part of the manifest.
pub const MANIFEST_FIELDS: &[FieldSpec] = &[];
/// One capability table entry.
pub const CAPABILITY_ENTRY_FIELDS: &[FieldSpec] = &[];
/// One directory-pointer table entry.
pub const DIRECTORY_ENTRY_FIELDS: &[FieldSpec] = &[];

/// Chapter 1 of `docs/CONTAINER.md`, rendered from the constants.
pub fn render_markdown() -> String {
    todo!("P4.1")
}
