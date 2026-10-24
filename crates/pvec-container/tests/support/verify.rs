//! A stand-in for the P4.5 reader and the P2.8 format-valid gate, neither of which exists yet: it
//! parses what the writer wrote through the header, manifest, directory and block parsers that do,
//! and checks the canonical order from the file alone.

use super::fixtures::*;
use pvec_capabilities::{Reader, Registry, Version};
use pvec_components::read_streams;
use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::advanced::{AdvancedView, BlockSlices};
use pvec_container::blocks::hot::HotBlockView;
use pvec_container::directory::{BlockKind, parse_directory};
use pvec_container::{DirectoryPointer, Header, Manifest};
use std::collections::BTreeSet;

pub struct Summary {
    pub blocks: usize,
    pub entities: usize,
}

pub fn verify(bytes: &[u8], input: &Input) -> Result<Summary, String> {
    let len = bytes.len() as u64;
    let header = Header::parse(&bytes[..bytes.len().min(4096)], len).map_err(|e| format!("header: {e}"))?;
    if header.manifest_offset != 64 {
        return Err(format!(
            "the manifest starts at {}, not straight after the header",
            header.manifest_offset
        ));
    }
    let m0 = header.manifest_offset as usize;
    let m1 = m0 + header.manifest_length as usize;
    let manifest = Manifest::parse(&bytes[m0..m1], &header, len).map_err(|e| format!("manifest: {e}"))?;
    if manifest.directories.len() != 1 || manifest.directories[0].kind != DirectoryPointer::KIND_BLOCK {
        return Err(format!("expected one block directory pointer, got {:?}", manifest.directories));
    }
    if manifest.capabilities.windows(2).any(|w| w[0].id >= w[1].id) {
        return Err("capabilities are not strictly ascending by id".to_string());
    }
    if manifest.metadata.producer_id != input.config.metadata.producer_id {
        return Err("producer id changed".to_string());
    }
    let ptr = manifest.directories[0];
    if ptr.offset + ptr.length != len {
        return Err(format!("the directory ends at {}, the file at {len}", ptr.offset + ptr.length));
    }
    let table = &bytes[ptr.offset as usize..(ptr.offset + ptr.length) as usize];
    let entries =
        parse_directory(table, header.header_len as u64, len).map_err(|e| format!("directory: {e}"))?;
    if entries.is_empty() {
        return Err("no blocks".to_string());
    }

    // canonical order, read back from the file
    let keys: Vec<(u16, u32, u16)> = entries.iter().map(|e| (e.kind, e.cell, e.subtype)).collect();
    if keys.windows(2).any(|w| w[0] >= w[1]) {
        return Err(format!(
            "blocks are not in strictly ascending (kind, cell, geometry type) order: {keys:?}"
        ));
    }

    // every byte outside a block and the directory is zero
    let mut at = m1;
    for e in &entries {
        let (start, end) = (e.offset as usize, e.offset as usize + e.stored_length as usize);
        if bytes[at..start].iter().any(|b| *b != 0) {
            return Err(format!("non-zero padding before the block at {start}"));
        }
        at = end;
    }
    if bytes[at..ptr.offset as usize].iter().any(|b| *b != 0) {
        return Err("non-zero padding before the directory".to_string());
    }

    let mut ids: BTreeSet<u32> = BTreeSet::new();
    let mut advanced: Vec<(u16, AlignedBytes)> = Vec::new();
    let mut record_count = 0;
    let mut stream_ids: Vec<u32> = Vec::new();
    let mut insert = |id: u32| -> Result<(), String> {
        if ids.insert(id) { Ok(()) } else { Err(format!("entity {id} is in two blocks")) }
    };
    for e in &entries {
        let raw = &bytes[e.offset as usize..e.offset as usize + e.stored_length as usize];
        let aligned = AlignedBytes::from_bytes(raw);
        match e.block_kind() {
            Some(BlockKind::Hot) => {
                let v = HotBlockView::parse(aligned.as_bytes()).map_err(|x| format!("hot block: {x}"))?;
                if v.kind().id() != e.subtype || v.len() != e.count as usize {
                    return Err("the directory entry disagrees with the hot block".to_string());
                }
                for id in v.entity_ids() {
                    insert(*id)?;
                }
            }
            Some(BlockKind::Records) | Some(BlockKind::Arena) => {
                if e.block_kind() == Some(BlockKind::Records) {
                    record_count = e.count as usize;
                }
                advanced.push(((e.kind << 8) | e.subtype, aligned));
            }
            Some(BlockKind::Components) => {
                let reader = Reader { version: Version::new(1, 0, 0), implemented: Vec::new() };
                let opened = read_streams(aligned.as_bytes(), &Registry::v1(), &reader)
                    .map_err(|x| format!("component set: {x:?}"))?;
                stream_ids = opened.skipped.iter().map(|s| s.capability.get()).collect();
            }
            None => return Err(format!("unknown block kind {}", e.kind)),
        }
    }
    if !advanced.is_empty() {
        let get = |k: u16, s: u16| {
            advanced
                .iter()
                .find(|(key, _)| *key == ((k << 8) | s))
                .map(|(_, b)| b.as_bytes())
                .ok_or(format!("no block {k}/{s}"))
        };
        let slices = BlockSlices {
            records: get(2, 0)?,
            knots: get(3, 1)?,
            weights: get(3, 2)?,
            control_points: get(3, 3)?,
            segments: get(3, 4)?,
            trims: get(3, 5)?,
        };
        let view = AdvancedView::parse(slices).map_err(|x| format!("advanced blocks: {x}"))?;
        view.check_nesting().map_err(|x| format!("nesting: {x}"))?;
        if view.len() != record_count {
            return Err("record count differs from the directory".to_string());
        }
        for i in 0..view.len() {
            let id = view.record(i).ok_or("record missing")?.id;
            insert(id)?;
            view.curve(i).map_err(|x| format!("curve {i}: {x}"))?;
        }
    }
    let wanted: BTreeSet<u32> = input.entities().map(Ent::id).collect();
    if ids != wanted {
        return Err(format!("the file holds {} entities, the input had {}", ids.len(), wanted.len()));
    }
    let mut want_streams: Vec<u32> = input.streams().map(|s| s.capability.get()).collect();
    want_streams.sort_unstable();
    if stream_ids != want_streams {
        return Err(format!("streams {stream_ids:?}, wanted {want_streams:?}"));
    }
    Ok(Summary { blocks: entries.len(), entities: ids.len() })
}
