//! Counts the exports of a `.wasm` file by kind, from its export section, so candidate (c) can say how many
//! entry points the WASM interface has next to the symbol counts of the C candidates.

use std::collections::BTreeMap;

fn leb(bytes: &[u8], at: &mut usize) -> usize {
    let (mut value, mut shift) = (0usize, 0);
    loop {
        let b = bytes[*at];
        *at += 1;
        value |= usize::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: p74-wasm-exports FILE.wasm");
    let bytes = std::fs::read(path).expect("read the module");
    assert_eq!(&bytes[..4], b"\0asm", "not a wasm module");
    let mut at = 8;
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    while at < bytes.len() {
        let id = bytes[at];
        at += 1;
        let size = leb(&bytes, &mut at);
        let end = at + size;
        if id == 7 {
            let count = leb(&bytes, &mut at);
            for _ in 0..count {
                let name_len = leb(&bytes, &mut at);
                let name = String::from_utf8_lossy(&bytes[at..at + name_len]).into_owned();
                at += name_len;
                let kind = match bytes[at] {
                    // `__wbindgen_*` and `__externref_*` are glue the toolchain adds, not entry points of ours.
                    0 if name.starts_with("__") => "function (toolchain glue)",
                    0 => "function (ours)",
                    1 => "table",
                    2 => "memory",
                    _ => "global",
                };
                at += 1;
                leb(&bytes, &mut at);
                *kinds.entry(kind).or_default() += 1;
            }
        }
        at = end;
    }
    for (kind, n) in kinds {
        println!("{kind} {n}");
    }
}
