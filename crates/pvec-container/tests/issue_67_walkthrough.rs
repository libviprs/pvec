//! Issue P8.8 (pvec#67): an implementer can use the specification alone to enumerate the required capabilities,
//! parse the header and skip an unknown optional block. The example in `docs/CONTAINER.md` chapter 6 is a real
//! file the writer produces today; the walkthrough below reads the bytes out of the document, not out of the
//! writer, and uses only the offsets the header, manifest and directory tables give.
#![allow(missing_docs)]

use pvec_capabilities::{CapabilityId, Version};
use pvec_components::Stream;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Manifest, Metadata};
use pvec_entity::EntityId;
use std::fs;
use std::path::PathBuf;

const BEGIN: &str = "<!-- example:begin -->";
const END: &str = "<!-- example:end -->";

/// The example file: one line and one component stream (an optional kind), unit mm, sealed.
fn example() -> Vec<u8> {
    let config = WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-doc-example".into(),
            producer_version: "1".into(),
            corpus_revision: None,
        },
        bounds: Some(Bounds { min: [0.0, 0.0], max: [10.0, 0.0] }),
        capabilities: vec![CapabilityId::new(14)],
    };
    let mut w = ContainerWriter::new(config);
    let meta = EntityMeta {
        id: EntityId::new(0).unwrap(),
        style: 0,
        cell: 0,
        bounds: Bounds { min: [0.0, 0.0], max: [10.0, 0.0] },
    };
    w.push_line(meta, [0.0, 0.0], [10.0, 0.0]).unwrap();
    w.push_stream(Stream { capability: CapabilityId::new(14), version: Version::new(1, 0, 0), payload: vec![0] })
        .unwrap();
    w.finish().unwrap()
}

fn dump(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, c)| {
            let hex: Vec<String> = c.iter().map(|b| format!("{b:02x}")).collect();
            let (a, b) = hex.split_at(hex.len().min(8));
            format!("{:08x}  {}  {}", i * 16, a.join(" "), b.join(" ")).trim_end().to_string()
        })
        .collect()
}

fn doc() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/CONTAINER.md");
    fs::read_to_string(path).expect("docs/CONTAINER.md")
}

/// The bytes of the dump in the document.
fn bytes_in_doc() -> Vec<u8> {
    let d = doc();
    let (from, to) = (d.find(BEGIN).expect("no example in CONTAINER.md"), d.find(END).expect("no end marker"));
    let mut out = Vec::new();
    for line in d[from + BEGIN.len()..to].lines() {
        let Some((offset, rest)) = line.split_once("  ") else { continue };
        if u64::from_str_radix(offset.trim(), 16).is_err() || offset.len() != 8 {
            continue;
        }
        assert_eq!(usize::from_str_radix(offset, 16).unwrap(), out.len(), "dump line out of order: {line}");
        out.extend(rest.split_whitespace().map(|h| u8::from_str_radix(h, 16).expect("a hex byte")));
    }
    out
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

#[test]
fn the_dump_in_the_document_is_the_file_the_writer_produces() {
    let file = example();
    if std::env::var_os("PVEC_BLESS_EXAMPLE").is_some() {
        println!("{}", dump(&file).join("\n"));
    }
    assert!(file.len() < 2048, "the example must stay small, it is {} bytes", file.len());
    assert_eq!(bytes_in_doc(), file, "the dump in docs/CONTAINER.md is not what the writer writes now");
}

#[test]
fn the_document_annotates_the_header_the_manifest_and_the_directory_of_the_dump() {
    let d = doc();
    let after = &d[d.find(END).expect("end marker")..];
    for needle in ["magic", "container_major", "manifest_offset", "capability", "block directory", "skip"] {
        assert!(after.contains(needle), "the annotation never mentions {needle:?}");
    }
}

#[test]
fn a_reader_built_from_the_tables_alone_lists_required_capabilities_and_skips_an_unknown_optional_block() {
    let f = bytes_in_doc();
    // header, section 1.2
    assert_eq!(&f[0..8], &[0x89, b'P', b'V', b'E', b'C', 0x0d, 0x0a, 0x1a], "magic");
    assert_eq!(u16_at(&f, 8), 1, "a reader that implements major 1 goes on");
    assert_eq!(u32_at(&f, 12), 0x0102_0304, "byte order marker, little endian");
    let header_len = u16_at(&f, 16) as usize;
    let (m_off, m_len) = (u64_at(&f, 24) as usize, u64_at(&f, 32) as usize);
    assert!(m_off >= header_len && m_off + m_len <= f.len());
    // manifest, section 1.3: capability entries are 16 bytes each, after the 128 byte fixed part
    let m = &f[m_off..m_off + m_len];
    let caps = u32_at(m, 4) as usize;
    let mut required = Vec::new();
    for i in 0..caps {
        let e = 128 + 16 * i;
        if m[e + 10] == 1 {
            required.push(u32_at(m, e));
        }
    }
    let reference = Manifest::parse(m, &pvec_container::Header::parse(&f[..header_len], f.len() as u64).unwrap(), f.len() as u64)
        .unwrap();
    let mut expected: Vec<u32> = reference
        .capabilities
        .iter()
        .filter(|c| c.requirement == pvec_capabilities::Requirement::Required)
        .map(|c| c.id.get())
        .collect();
    expected.sort_unstable();
    required.sort_unstable();
    assert_eq!(required, expected, "the required capabilities read from the tables are the writer's");
    // directory pointer, section 1.3: kind 1 is the block directory
    let table = m_off + 128 + 16 * caps;
    assert_eq!(u32_at(&f, table), 1, "the first directory pointer is the block directory");
    let (d_off, d_len) = (u64_at(&f, table + 8) as usize, u64_at(&f, table + 16) as usize);
    assert_eq!(d_len % 64, 0);
    // block directory, section 4.2: a reader that knows only hot blocks (kind 1)
    let (mut kept, mut skipped) = (0, 0);
    for e in f[d_off..d_off + d_len].chunks(64) {
        let (kind, flags) = (u16_at(e, 56), e[60]);
        if kind == 1 {
            kept += 1;
        } else if flags & 1 == 0 {
            let (offset, stored) = (u64_at(e, 0), u32_at(e, 8));
            assert!(offset as usize + stored as usize <= f.len(), "the skip is computed from the descriptor alone");
            skipped += 1;
        } else {
            panic!("a must-understand block of kind {kind} would refuse the file");
        }
    }
    assert!(kept >= 1 && skipped >= 1, "the example has a block to read and an optional one to skip");
}
