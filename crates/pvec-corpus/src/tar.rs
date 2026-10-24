//! A reader for the deterministic ustar bundle: regular files only, every size and name checked.

use crate::error::{CorpusError, Result};

/// One file of the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// The path inside the archive, with `/` separators.
    pub path: String,
    /// The file's bytes.
    pub data: &'a [u8],
}

/// Upper bound on entries, so a hostile header run cannot size an allocation.
pub const MAX_ENTRIES: usize = 4096;

const BLOCK: usize = 512;

fn bad(why: impl Into<String>) -> CorpusError {
    CorpusError::Tar(why.into())
}

/// An octal field: digits, then NULs or spaces.
fn octal(field: &[u8], what: &str) -> Result<u64> {
    let text: Vec<u8> = field.iter().copied().take_while(|b| *b != 0 && *b != b' ').collect();
    if text.is_empty() || !text.iter().all(|b| (b'0'..=b'7').contains(b)) {
        return Err(bad(format!("{what} is not an octal number")));
    }
    let mut n: u64 = 0;
    for b in text {
        n = n
            .checked_mul(8)
            .and_then(|n| n.checked_add(u64::from(b - b'0')))
            .ok_or_else(|| bad(format!("{what} overflows")))?;
    }
    Ok(n)
}

fn text(field: &[u8]) -> &[u8] {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    &field[..end]
}

fn check_path(path: &str) -> Result<()> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return Err(bad(format!("path `{path}` is empty, absolute or uses a backslash")));
    }
    if path.split('/').any(|c| c.is_empty() || c == "." || c == "..") {
        return Err(bad(format!("path `{path}` has an empty, `.` or `..` component")));
    }
    Ok(())
}

/// Parses a tar held in memory.
pub fn parse(bytes: &[u8]) -> Result<Vec<Entry<'_>>> {
    let mut entries: Vec<Entry<'_>> = Vec::new();
    let mut pos = 0usize;
    loop {
        let header = bytes
            .get(pos..pos + BLOCK)
            .ok_or_else(|| bad("truncated header block, or no end-of-archive marker"))?;
        if header.iter().all(|b| *b == 0) {
            let next = bytes
                .get(pos + BLOCK..pos + 2 * BLOCK)
                .ok_or_else(|| bad("the end-of-archive marker is one block long"))?;
            if !next.iter().all(|b| *b == 0) || !bytes[pos + 2 * BLOCK..].iter().all(|b| *b == 0) {
                return Err(bad("data after the end-of-archive marker"));
            }
            return Ok(entries);
        }
        if entries.len() >= MAX_ENTRIES {
            return Err(bad(format!("more than {MAX_ENTRIES} entries")));
        }
        let recorded = octal(&header[148..156], "checksum")?;
        let computed: u64 = header
            .iter()
            .enumerate()
            .map(|(i, b)| if (148..156).contains(&i) { u64::from(b' ') } else { u64::from(*b) })
            .sum();
        if recorded != computed {
            return Err(bad(format!("header checksum {recorded} does not match the computed {computed}")));
        }
        match header[156] {
            b'0' | 0 => {}
            other => return Err(bad(format!("entry type `{}` is not a regular file", other as char))),
        }
        let size =
            usize::try_from(octal(&header[124..136], "size")?).map_err(|_| bad("size does not fit"))?;
        let mut path = String::new();
        if &header[257..263] == b"ustar\0" {
            let prefix =
                std::str::from_utf8(text(&header[345..500])).map_err(|_| bad("prefix is not UTF-8"))?;
            if !prefix.is_empty() {
                path.push_str(prefix);
                path.push('/');
            }
        }
        path.push_str(std::str::from_utf8(text(&header[..100])).map_err(|_| bad("name is not UTF-8"))?);
        check_path(&path)?;
        if entries.iter().any(|e| e.path == path) {
            return Err(bad(format!("duplicate path `{path}`")));
        }
        let start = pos + BLOCK;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| bad(format!("`{path}` runs past the end of the archive")))?;
        entries.push(Entry { path, data: &bytes[start..end] });
        pos = start + size.div_ceil(BLOCK) * BLOCK;
    }
}
