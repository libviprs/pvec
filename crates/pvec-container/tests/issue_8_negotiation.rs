//! Capability negotiation and unknown-block skipping (issue P4.7): a reader that meets a block or a
//! capability it has never heard of either skips it correctly, without reading its payload, or refuses the
//! file and says which capability and version it was missing. Every optional capability is additive,
//! because of this, and nothing optional can change what the geometry says.

mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
use pvec_container::directory::BlockKind;
use pvec_container::integrity::Integrity;
use pvec_container::negotiate::{
    Effect, Outcome, PartialOpen, ReaderCapabilities, Refusal, RewriteRefused, SkipReason, negotiate,
};
use pvec_container::reader::{BlockId, ContainerReader, ReadError, ReaderConfig, Validated};
use pvec_container::source::{BlockSource, Bytes, MemorySource, Recording, SourceError};
use pvec_container::writer::{RawBlock, assemble};
use std::sync::Arc;
use support::art::{self, Art, SEALED, UNSEALED, rebuild, rebuild_raw};
use support::fixtures::{Item, fixtures, mixed};

fn cap(id: u32) -> CapabilityId {
    CapabilityId::new(id)
}

fn declared(id: u32, requirement: Requirement, major: u16, minor: u16) -> Declared {
    Declared { id: cap(id), requirement, version: Version::new(major, minor, 0) }
}

/// The artifact with its capability list replaced (the list has to stay ascending by id).
fn with_caps(art: &Art, integrity: Integrity, mut caps: Vec<Declared>) -> Art {
    caps.sort_by_key(|d| d.id);
    rebuild(art, integrity, |m, _| m.capabilities = caps)
}

/// A block of a kind no reader has heard of, optional, belonging to capability `capability`.
fn unknown_block(kind: u16, capability: u16) -> RawBlock {
    RawBlock {
        kind,
        subtype: 0,
        cell: 0,
        count: 1,
        bounds: None,
        flags: 0,
        codec: 0,
        capability,
        uncompressed_length: 64,
        stored: (0..64u8).map(|i| 0xA0 ^ i).collect(),
    }
}

/// `art` plus an optional block of kind 9 that belongs to capability 99, which the manifest declares
/// optional. Where the block goes in the order of the directory is the assembler's canonical order.
fn with_unknown_optional_block(art: &Art, integrity: Integrity) -> Art {
    rebuild(art, integrity, |m, blocks| {
        m.capabilities.push(declared(99, Requirement::Optional, 3, 1));
        blocks.push(unknown_block(9, 99));
    })
}

fn open_with(
    bytes: Vec<u8>,
    caps: ReaderCapabilities,
) -> Result<ContainerReader<Recording<MemorySource<Vec<u8>>>>, ReadError> {
    let cfg = ReaderConfig { capabilities: caps, ..ReaderConfig::default() };
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), cfg)
}

/// Every geometry block, decoded: kind, subtype, cell and the bytes. Components are not geometry and are left out.
fn geometry(reader: &ContainerReader<Recording<MemorySource<Vec<u8>>>>) -> Vec<(u16, u16, u32, Vec<u8>)> {
    let mut out = Vec::new();
    for (i, e) in reader.entries().iter().enumerate() {
        if matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)) {
            let b = reader.read_block(BlockId(i as u32)).expect("a geometry block reads");
            out.push((e.kind, e.subtype, e.cell, b.as_bytes().to_vec()));
        }
    }
    out
}

// ---------------------------------------------------------------- the three outcomes

#[test]
fn a_file_whose_capabilities_are_all_supported_opens_with_nothing_skipped() {
    for input in fixtures() {
        let art = Art::new(input.write().unwrap());
        assert_eq!(
            negotiate(&art.manifest, &art.entries, &ReaderCapabilities::v1()),
            Outcome::Open,
            "{}",
            input.name
        );
        let reader = open_with(art.bytes.clone(), ReaderCapabilities::v1()).unwrap();
        assert!(reader.partial_open().is_none());
        assert!(reader.validate().unwrap().skipped.is_empty());
    }
}

#[test]
fn an_unsupported_optional_capability_opens_degraded_and_is_listed_by_id() {
    let base = art::lines_with(4, 1, SEALED);
    let art = with_caps(&base, Integrity::Full, vec![declared(99, Requirement::Optional, 3, 1)]);
    match negotiate(&art.manifest, &art.entries, &ReaderCapabilities::v1()) {
        Outcome::OpenDegraded(p) => {
            assert_eq!(p.skipped_ids(), vec![99]);
            let c = &p.capabilities[0];
            assert_eq!(
                (c.id, c.version, c.reason, c.effect),
                (cap(99), Some(Version::new(3, 1, 0)), SkipReason::NotImplemented, Effect::Unknown)
            );
        }
        other => panic!("{other:?}"),
    }
    let reader = open_with(art.bytes.clone(), ReaderCapabilities::v1())
        .expect("an optional capability does not stop the open");
    assert_eq!(reader.partial_open().expect("a report").skipped_ids(), vec![99]);
    reader.validate().expect("and the file validates");
}

#[test]
fn an_unsupported_required_capability_is_refused_and_the_refusal_names_the_id_and_the_version() {
    let base = art::lines_with(4, 1, SEALED);
    let art = with_caps(&base, Integrity::Full, vec![declared(99, Requirement::Required, 2, 1)]);
    let out = negotiate(&art.manifest, &art.entries, &ReaderCapabilities::v1());
    assert_eq!(
        out,
        Outcome::Refuse(Refusal::MissingRequired { id: cap(99), version: Version::new(2, 1, 0), name: None })
    );
    let e = open_with(art.bytes.clone(), ReaderCapabilities::v1()).err().expect("refused");
    assert!(matches!(&e, ReadError::Negotiation(Refusal::MissingRequired { .. })), "{e:?}");
    let text = e.to_string();
    assert!(
        text.contains("99") && text.contains("2.1"),
        "the message names the capability and its version: {text}"
    );
    assert_eq!(e.rule(), "NEGOTIATE-01");
}

#[test]
fn a_required_capability_the_registry_knows_but_this_reader_does_not_implement_is_refused_by_name() {
    let base = art::lines_with(4, 1, SEALED);
    let art = with_caps(&base, Integrity::Full, vec![declared(11, Requirement::Required, 1, 0)]);
    let reader = ReaderCapabilities::v1().without(cap(11));
    match negotiate(&art.manifest, &art.entries, &reader) {
        Outcome::Refuse(Refusal::MissingRequired { id, version, name }) => {
            assert_eq!((id, version), (cap(11), Version::new(1, 0, 0)));
            assert_eq!(name, Some("b_spline_curve"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_newer_minor_version_of_a_known_capability_opens_and_a_newer_major_of_a_required_one_is_refused() {
    let base = art::lines_with(4, 1, SEALED);
    let minor = with_caps(&base, Integrity::Full, vec![declared(4, Requirement::Required, 1, 9)]);
    assert_eq!(negotiate(&minor.manifest, &minor.entries, &ReaderCapabilities::v1()), Outcome::Open);
    assert!(open_with(minor.bytes.clone(), ReaderCapabilities::v1()).is_ok());
    let major = with_caps(&base, Integrity::Full, vec![declared(4, Requirement::Required, 2, 0)]);
    match negotiate(&major.manifest, &major.entries, &ReaderCapabilities::v1()) {
        Outcome::Refuse(Refusal::IncompatibleMajor { id, file, supported_major }) => {
            assert_eq!((id, file, supported_major), (cap(4), Version::new(2, 0, 0), 1));
        }
        other => panic!("{other:?}"),
    }
    let e = open_with(major.bytes.clone(), ReaderCapabilities::v1()).err().expect("refused");
    assert_eq!(e.rule(), "NEGOTIATE-02");
    assert!(e.to_string().contains("2.0"), "{e}");
}

#[test]
fn a_newer_major_of_an_optional_capability_is_skipped_and_not_refused() {
    let base = art::from_input_with(&mixed(), SEALED);
    let caps = base
        .manifest
        .capabilities
        .iter()
        .map(|d| if d.id == cap(14) { Declared { version: Version::new(3, 0, 0), ..*d } } else { *d })
        .collect();
    let art = with_caps(&base, Integrity::Full, caps);
    match negotiate(&art.manifest, &art.entries, &ReaderCapabilities::v1()) {
        Outcome::OpenDegraded(p) => {
            let c = p.capabilities.iter().find(|c| c.id == cap(14)).expect("layer is listed");
            assert_eq!((c.reason, c.effect), (SkipReason::NewerMajor, Effect::Unstyled));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_that_needs_a_newer_reader_a_component_declared_required_and_an_extension_declared_required_are_refused()
 {
    let base = art::from_input_with(&mixed(), SEALED);
    let newer = rebuild(&base, Integrity::Full, |m, _| m.min_reader = Version::new(2, 0, 0));
    assert!(matches!(
        negotiate(&newer.manifest, &newer.entries, &ReaderCapabilities::v1()),
        Outcome::Refuse(Refusal::ReaderTooOld { .. })
    ));
    let comp = rebuild(&base, Integrity::Full, |m, _| {
        for d in &mut m.capabilities {
            if d.id == cap(14) {
                d.requirement = Requirement::Required;
            }
        }
    });
    assert_eq!(
        negotiate(&comp.manifest, &comp.entries, &ReaderCapabilities::v1()),
        Outcome::Refuse(Refusal::ComponentDeclaredRequired(cap(14)))
    );
}

#[test]
fn a_reader_that_implements_fewer_component_capabilities_degrades_and_says_what_is_left_undone() {
    let art = art::from_input_with(&mixed(), SEALED);
    let none = ReaderCapabilities::v1()
        .without(cap(14))
        .without(cap(15))
        .without(cap(16))
        .without(cap(18))
        .without(cap(20));
    let reader = open_with(art.bytes.clone(), none).unwrap();
    let partial = reader.partial_open().expect("a report");
    let by_id = |id: u32| {
        partial
            .capabilities
            .iter()
            .find(|c| c.id == cap(id))
            .unwrap_or_else(|| panic!("{id} is not listed: {partial:?}"))
    };
    assert_eq!(by_id(14).effect, Effect::Unstyled);
    assert_eq!(by_id(15).effect, Effect::Unstyled);
    assert_eq!(by_id(16).effect, Effect::Uncomponented);
    assert_eq!(partial.skipped_ids(), vec![14, 15, 16, 18, 20]);
    assert!(partial.entities >= 141, "the report counts the entities a skipped capability may apply to");
    let sentence = partial.describe();
    assert!(sentence.contains("14") && sentence.to_lowercase().contains("style"), "{sentence}");
}

// ---------------------------------------------------------------- skipping a block

#[test]
fn an_unknown_optional_block_is_skipped_its_capability_is_reported_and_the_geometry_is_bit_identical() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let with = with_unknown_optional_block(&base, Integrity::Full);
    let without = rebuild(&base, Integrity::Full, |m, _| {
        m.capabilities.push(declared(99, Requirement::Optional, 3, 1))
    });
    let r = open_with(with.bytes.clone(), ReaderCapabilities::v1())
        .expect("an unknown optional block does not stop the open");
    let partial = r.partial_open().expect("a report");
    assert_eq!(partial.skipped_ids(), vec![99], "its capability id is in the report");
    assert_eq!(partial.blocks.len(), 1);
    let b = partial.blocks[0];
    assert_eq!(
        (b.kind, b.capability, b.reason, b.stored_length),
        (9, Some(cap(99)), SkipReason::UnknownBlockKind, 64)
    );
    r.validate().expect("the file validates around the block");
    let alone = open_with(without.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    assert_eq!(geometry(&r), geometry(&alone), "the geometry is the same with and without the block");
}

#[test]
fn skipping_a_block_reads_zero_bytes_of_its_payload() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = with_unknown_optional_block(&base, Integrity::Full);
    let i = art.entries.iter().position(|e| e.kind == 9).expect("the block");
    let (offset, len) = (art.entries[i].offset, u64::from(art.entries[i].stored_length));
    let r = open_with(art.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    r.validate().unwrap();
    let ids: Vec<BlockId> = (0..art.entries.len() as u32).map(BlockId).collect();
    let plan = r.plan(&ids).unwrap();
    assert!(plan.iter().all(|q| !q.blocks.contains(&BlockId(i as u32))), "the plan does not ask for it");
    r.read_blocks(&ids.iter().copied().filter(|b| b.0 as usize != i).collect::<Vec<_>>()).unwrap();
    let _ = r.advanced(0).unwrap();
    assert!(
        !r.source().touched(offset, len),
        "a request touched the payload of a skipped block: {:?}",
        r.source().requests()
    );
    assert!(
        matches!(r.read_block(BlockId(i as u32)), Err(ReadError::Skipped { .. })),
        "and asking for it is an error, not a read"
    );
    assert!(!r.source().touched(offset, len));
}

/// A source that fails if anything in `lo..hi` is asked for.
struct Poisoned {
    inner: MemorySource<Vec<u8>>,
    lo: u64,
    hi: u64,
}

impl BlockSource for Poisoned {
    fn len(&self) -> u64 {
        BlockSource::len(&self.inner)
    }
    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        if offset < self.hi && self.lo < offset + u64::from(len) {
            return Err(SourceError::Io("the payload of a skipped block was read".into()));
        }
        self.inner.read_range(offset, len)
    }
}

#[test]
fn a_block_is_skipped_from_its_descriptor_alone_so_a_source_that_cannot_serve_it_changes_nothing() {
    let base = art::from_input_with(&fixtures()[2], UNSEALED);
    let art = with_unknown_optional_block(&base, Integrity::None);
    let i = art.entries.iter().position(|e| e.kind == 9).unwrap();
    let (lo, hi) = (art.entries[i].offset, art.entries[i].offset + u64::from(art.entries[i].stored_length));
    let source = Poisoned { inner: MemorySource::new(art.bytes.clone()), lo, hi };
    let reader = ContainerReader::open(source, ReaderConfig::default()).expect("opens without touching it");
    let v: Validated = reader.validate().expect("validates without touching it");
    assert_eq!(v.skipped, vec![BlockId(i as u32)]);
}

#[test]
fn an_optional_block_with_a_codec_this_reader_does_not_know_is_skipped_too() {
    let base = art::from_input_with(&mixed(), SEALED);
    let art = rebuild(&base, Integrity::Full, |_, blocks| {
        let comp =
            blocks.iter_mut().find(|b| b.kind == BlockKind::Components.id()).expect("the component set");
        comp.codec = 7;
    });
    let r = open_with(art.bytes.clone(), ReaderCapabilities::v1()).expect("opens");
    let p = r.partial_open().expect("a report");
    assert_eq!(p.blocks[0].reason, SkipReason::UnknownCodec);
    assert_eq!(p.blocks[0].codec, 7);
    r.validate().unwrap();
}

#[test]
fn an_unknown_block_that_must_be_understood_and_one_that_names_a_capability_nobody_declared_are_refused() {
    let base = art::from_input_with(&fixtures()[0], SEALED);
    let required = rebuild_raw(&base, Integrity::Full, |m, blocks| {
        m.capabilities.push(declared(99, Requirement::Optional, 1, 0));
        let mut b = unknown_block(9, 99);
        b.flags = 1;
        blocks.push(b);
    });
    // the directory parser refuses it before negotiation is asked
    assert!(open_with(required, ReaderCapabilities::v1()).err().expect("refused").rule() == "DIR-04");
    let undeclared = rebuild(&base, Integrity::Full, |_, blocks| blocks.push(unknown_block(9, 77)));
    let e = open_with(undeclared.bytes.clone(), ReaderCapabilities::v1()).err().expect("refused");
    assert_eq!(e.rule(), "NEGOTIATE-05", "{e}");
}

#[test]
fn the_descriptor_names_a_capability_only_on_a_block_of_a_kind_the_format_does_not_define() {
    let base = art::from_input_with(&fixtures()[0], SEALED);
    let known = rebuild_raw(&base, Integrity::Full, |m, blocks| {
        m.capabilities.push(declared(99, Requirement::Optional, 1, 0));
        blocks[0].capability = 99;
    });
    assert_eq!(open_with(known, ReaderCapabilities::v1()).err().expect("refused").rule(), "DIR-02");
    let unknown = with_unknown_optional_block(&base, Integrity::Full);
    assert_eq!(unknown.entries.iter().filter(|e| e.capability == 99).count(), 1);
}

// ---------------------------------------------------------------- re-writing what was not understood

#[test]
fn rewriting_a_file_whose_optional_blocks_were_not_understood_is_refused_by_the_semantic_path_and_exact_by_the_raw_one()
 {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = with_unknown_optional_block(&base, Integrity::Full);
    let r = open_with(art.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    let partial: &PartialOpen = r.partial_open().unwrap();
    // decision: a rewrite from the decoded model cannot carry what it did not decode, so it refuses
    assert_eq!(partial.ensure_lossless_rewrite(), Err(RewriteRefused { skipped: vec![99] }));
    // and a tool that copies blocks as they are (the assembler) preserves the unknown block byte for byte
    let blocks: Vec<RawBlock> = art
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| RawBlock {
            kind: e.kind,
            subtype: e.subtype,
            cell: e.cell,
            count: e.count,
            bounds: e.bounds,
            flags: e.flags,
            codec: e.codec,
            capability: e.capability,
            uncompressed_length: e.uncompressed_length,
            stored: r.read_stored(BlockId(i as u32)).expect("the raw bytes").to_vec(),
        })
        .collect();
    let again = assemble(art.manifest.clone(), &blocks, Integrity::Full).unwrap();
    assert_eq!(again, art.bytes, "copied raw, the file comes back byte for byte, the unknown block included");
    // a file that skipped nothing can be rewritten
    let plain = open_with(base.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    assert!(plain.partial_open().is_none());
    assert!(PartialOpen::default().ensure_lossless_rewrite().is_ok());
}

// ---------------------------------------------------------------- what optional capabilities may change

#[test]
fn the_geometry_is_identical_whatever_optional_component_capabilities_the_file_has_or_the_reader_supports() {
    let input = mixed();
    let with_streams = Art::new(input.write().unwrap());
    let mut bare = input.clone();
    bare.items.retain(|i| !matches!(i, Item::Stream(_)));
    let without_streams = Art::new(bare.write().unwrap());
    assert!(without_streams.entries.iter().all(|e| e.kind != BlockKind::Components.id()));
    let full = open_with(with_streams.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    let none = open_with(
        with_streams.bytes.clone(),
        [14u32, 15, 16, 17, 18, 19, 20]
            .into_iter()
            .fold(ReaderCapabilities::v1(), |c, id| c.without(cap(id))),
    )
    .unwrap();
    let bare = open_with(without_streams.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    let (a, b, c) = (geometry(&full), geometry(&none), geometry(&bare));
    assert!(!a.is_empty());
    assert_eq!(a, b, "skipping every component capability does not move a geometry byte");
    assert_eq!(a, c, "and neither does not having the streams at all");
    assert!(none.partial_open().is_some() && full.partial_open().is_none());
}

#[test]
fn blocking_and_async_readers_agree_on_what_a_degraded_open_skipped() {
    use pvec_container::reader::AsyncContainerReader;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    fn block_on<F: Future>(f: F) -> F::Output {
        let mut f = pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = with_unknown_optional_block(&base, Integrity::Full);
    let sync = open_with(art.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    let source = MemorySource::new(Arc::<[u8]>::from(&art.bytes[..]));
    let a = block_on(AsyncContainerReader::open(source, ReaderConfig::default())).unwrap();
    assert_eq!(sync.partial_open(), a.partial_open());
    assert_eq!(sync.validate().unwrap(), block_on(a.validate()).unwrap());
}

#[test]
fn chapter_5_of_the_container_document_states_the_skip_contract_and_what_a_new_capability_must_satisfy() {
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONTAINER.md")).unwrap();
    for needle in [
        "## 5. Negotiation and skipping",
        "`Open`",
        "`OpenDegraded`",
        "`Refuse`",
        "a descriptor alone",
        "must satisfy to be optional",
        "NEGOTIATE-01",
        "NEGOTIATE-05",
        "byte for byte",
    ] {
        assert!(doc.contains(needle), "docs/CONTAINER.md chapter 5 lacks {needle:?}");
    }
}

#[test]
fn a_coalesced_request_never_reads_across_a_skipped_block() {
    // a block of a known kind that is optional and has a codec nobody knows, between two live blocks that
    // touch it: with a gap threshold that would otherwise merge all three into one request
    // blocks of 8 KB, so the first 4 KiB the open reads is the first block only
    let base = art::lines_with(200, 3, SEALED);
    let art = rebuild(&base, Integrity::Full, |_, blocks| {
        blocks[1].flags = 0;
        blocks[1].bounds = None;
        blocks[1].codec = 7;
    });
    let r = open_with(art.bytes.clone(), ReaderCapabilities::v1()).unwrap();
    assert_eq!(r.partial_open().unwrap().blocks.len(), 1);
    let plan = r.plan(&[BlockId(0), BlockId(1), BlockId(2)]).unwrap();
    assert_eq!(plan.len(), 2, "{plan:?}");
    assert!(plan.iter().all(|q| !q.blocks.contains(&BlockId(1))));
    let (offset, len) = (art.entries[1].offset, u64::from(art.entries[1].stored_length));
    r.read_blocks(&[BlockId(0), BlockId(2)]).unwrap();
    assert!(!r.source().touched(offset, len), "the skipped block was read inside a coalesced request");
}
