//! The cases. One named case per attack, each a mutation of a valid base.

use super::bases::Base;
use super::mutate::{BlockLoc, Layout, Op, apply, entry_field, field_at};
use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::advanced::{RECORD_FIELDS, RECORD_HEADER_FIELDS, RECORD_HEADER_LEN, RECORD_LEN};
use pvec_container::blocks::arena::{ARENA_HEADER_FIELDS, ARENA_HEADER_LEN, OFFSET_ENTRY_LEN};
use pvec_container::blocks::hot::{HOT_HEADER_FIELDS, HotBlockView};
use pvec_container::directory::ENTRY_LEN;
use pvec_container::spec::{cap, dir, h, m};
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

/// What a case needs from a reader beyond reading the parts of the format that already have parsers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Requires {
    /// Only what the header, manifest, directory and block parsers refuse.
    Parse,
    /// A resource budget (P4.8): an expansion ratio, a decoded-size ceiling.
    Limits,
    /// Per-block checksums and the artifact digest (P4.6).
    Integrity,
    /// Skipping an optional block the reader does not know (P4.7).
    Negotiation,
    /// The numeric policy applied to the values of a hot block (P4.5). The hot parsers refuse a NaN
    /// or an infinity (`HOT-16`) and nothing else: a radius of zero, a coordinate past 2^40 and a
    /// zero-length line all parse, so a safe reader has to check them as it uses them.
    Semantics,
}

/// The case families of the issue, plus the ones it implies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Family {
    /// Offset and length wrap, past the end of the file, overlapping blocks.
    Offsets,
    /// Entity, control-point, knot and record counts that lie.
    Counts,
    /// An expansion ratio beyond the budget.
    Decompression,
    /// A composite that names itself, and nesting that is too deep.
    Nesting,
    /// Knot vectors that are not monotonic, the wrong length, or repeat too often.
    Knots,
    /// Zero, negative, non-finite or missing weights.
    Weights,
    /// Inverted, zero-length or non-finite domains and coordinates.
    Domains,
    /// A wrong artifact digest or block checksum.
    Integrity,
    /// A file cut short, or with a byte removed.
    Truncation,
    /// Bytes after the end of the artifact.
    TrailingBytes,
    /// A kind, codec, version, flag or algorithm the reader does not know.
    UnknownTags,
    /// Order, padding, reserved fields and length fields that are wrong inside a block.
    Corruption,
}

/// What a reader has to do with the mutated file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    /// Refuse, naming one of these rules.
    Refuse(Vec<&'static str>),
    /// Accept: the mutation is something a reader must tolerate.
    Accept,
}

/// One attack.
#[derive(Clone, Debug)]
pub struct Case {
    /// A unique name, `family/what`, or `family/what/base/position` for the generated sweeps.
    pub name: String,
    /// The family.
    pub family: Family,
    /// What a reader needs to provide to be held to it.
    pub requires: Requires,
    /// Index into the bases.
    pub base: usize,
    /// The mutation, as byte-level edits of the base.
    pub ops: Vec<Op>,
    /// What the reader has to do.
    pub expect: Expect,
}

impl Case {
    /// The mutated file.
    pub fn bytes(&self, base: &Base) -> Vec<u8> {
        apply(&base.bytes, &self.ops)
    }
}

// ---------------------------------------------------------------- locating things in a base

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// An arena block: where its table and its columns are.
#[derive(Clone, Copy)]
struct Arena {
    block: usize,
    entries: usize,
    elements: usize,
}

impl Arena {
    fn header(&self, name: &str) -> usize {
        self.block + field_at(ARENA_HEADER_FIELDS, name)
    }
    /// Absolute offset of table entry `k` (`start: u32`, then `len: u32`).
    fn entry(&self, k: usize) -> usize {
        self.block + ARENA_HEADER_LEN + OFFSET_ENTRY_LEN * k
    }
    /// Absolute offset of element `e` of the first column, `width` bytes wide.
    fn column(&self, e: usize, width: usize) -> usize {
        self.block + ARENA_HEADER_LEN + OFFSET_ENTRY_LEN * self.entries + width * e
    }
    /// Absolute offset of element `e` of the second `f64` column (control points: `y`).
    fn second_column(&self, e: usize) -> usize {
        self.column(self.elements + e, 8)
    }
}

/// A valid base and where the parts of it are.
struct At<'a> {
    bytes: &'a [u8],
    layout: Layout,
}

impl<'a> At<'a> {
    fn new(bytes: &'a [u8]) -> At<'a> {
        At { bytes, layout: Layout::of(bytes) }
    }

    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn block(&self, kind: u16, subtype: u16) -> BlockLoc {
        *self.layout.block(kind, subtype).unwrap_or_else(|| panic!("no block {kind}/{subtype}"))
    }

    fn hot_field(&self, geometry: u16, name: &str) -> usize {
        self.block(1, geometry).offset + field_at(HOT_HEADER_FIELDS, name)
    }

    /// Absolute offsets of the parts of a hot block: the ids, the run offsets (polylines) and the `f64` columns.
    fn hot_parts(&self, geometry: u16) -> (usize, usize, Vec<usize>) {
        let loc = self.block(1, geometry);
        let copy = AlignedBytes::from_bytes(&self.bytes[loc.offset..loc.offset + loc.len]);
        let base = copy.as_bytes().as_ptr() as usize;
        let v = HotBlockView::parse(copy.as_bytes()).expect("a valid hot block");
        // an empty slice (the run offsets of a block with no runs) points nowhere in the block
        let off = |p: usize| if p >= base && p - base < loc.len { loc.offset + (p - base) } else { 0 };
        let cols = v.f64_columns().iter().map(|c| off(c.as_ptr() as usize)).collect();
        (off(v.entity_ids().as_ptr() as usize), off(v.run_offsets().as_ptr() as usize), cols)
    }

    fn arena(&self, payload: u16) -> Arena {
        let b = self.block(3, payload).offset;
        Arena {
            block: b,
            entries: u32_at(self.bytes, b + field_at(ARENA_HEADER_FIELDS, "entry_count")) as usize,
            elements: u32_at(self.bytes, b + field_at(ARENA_HEADER_FIELDS, "element_count")) as usize,
        }
    }

    /// Absolute offset of a field of record `index`.
    fn record(&self, index: usize, field: &str) -> usize {
        self.block(2, 0).offset + RECORD_HEADER_LEN + RECORD_LEN * index + field_at(RECORD_FIELDS, field)
    }

    /// Absolute offset of the table entry a record field names, and the `(start, len)` in it.
    fn run(&self, payload: u16, record: usize, field: &str) -> (usize, (usize, usize)) {
        let k = u32_at(self.bytes, self.record(record, field)) as usize;
        let at = self.arena(payload).entry(k);
        (at, (u32_at(self.bytes, at) as usize, u32_at(self.bytes, at + 4) as usize))
    }

    /// Absolute offset of knot `i` of a spline record.
    fn knot(&self, record: usize, i: usize) -> usize {
        let (_, (start, len)) = self.run(KNOTS, record, "knots");
        assert!(i < len, "knot {i} of {len}");
        self.arena(KNOTS).column(start + i, 8)
    }

    fn knots(&self, record: usize) -> (usize, usize) {
        let (_, (start, len)) = self.run(KNOTS, record, "knots");
        (self.arena(KNOTS).column(start, 8), len)
    }

    fn knot_values(&self, record: usize) -> Vec<f64> {
        let (at, len) = self.knots(record);
        (0..len)
            .map(|i| {
                f64::from_bits(u64::from_le_bytes(
                    self.bytes[at + 8 * i..at + 8 * i + 8].try_into().expect("8"),
                ))
            })
            .collect()
    }

    fn manifest(&self, field: usize) -> usize {
        self.layout.manifest + field
    }
}

const KNOTS: u16 = 1;
const WEIGHTS: u16 = 2;
const CONTROL_POINTS: u16 = 3;
const SEGMENTS: u16 = 4;
const TRIMS: u16 = 5;

const LINE: u16 = 1;
const POLYLINE: u16 = 2;
const CIRCLE_ARC: u16 = 3;

// ---------------------------------------------------------------- the catalog

struct Builder<'a> {
    bases: &'a [Base],
    cases: Vec<Case>,
}

impl Builder<'_> {
    fn add(
        &mut self,
        family: Family,
        requires: Requires,
        name: &str,
        base: &str,
        ops: Vec<Op>,
        expect: Expect,
    ) {
        let prefix = match family {
            Family::Offsets => "offsets",
            Family::Counts => "counts",
            Family::Decompression => "decompression",
            Family::Nesting => "nesting",
            Family::Knots => "knots",
            Family::Weights => "weights",
            Family::Domains => "domains",
            Family::Integrity => "integrity",
            Family::Truncation => "truncation",
            Family::TrailingBytes => "trailing",
            Family::UnknownTags => "tags",
            Family::Corruption => "corruption",
        };
        let base = self.bases.iter().position(|b| b.name == base).unwrap_or_else(|| panic!("no base {base}"));
        self.cases.push(Case { name: format!("{prefix}/{name}"), family, requires, base, ops, expect });
    }

    /// A case that needs only the parsers and wants a refusal naming one of `rules`.
    fn refuse(&mut self, family: Family, name: &str, base: &str, ops: Vec<Op>, rules: &[&'static str]) {
        self.add(family, Requires::Parse, name, base, ops, Expect::Refuse(rules.to_vec()));
    }
}

fn nan(at: usize) -> Op {
    Op::f64(at, f64::NAN)
}

fn inf(at: usize) -> Op {
    Op::f64(at, f64::INFINITY)
}

/// Every case over these bases: `bases` is what [`super::bases::bases`] returned.
///
/// The named cases come first, then the sweeps: every prefix of every base and every base with one
/// byte removed, minus the ones a named case already is.
pub fn catalog(bases: &[Base]) -> Vec<Case> {
    let mut b = Builder { bases, cases: Vec::new() };
    let get = |name: &str| {
        let i = bases.iter().position(|x| x.name == name).unwrap_or_else(|| panic!("no base {name}"));
        At::new(&bases[i].bytes)
    };
    let drawing = get("drawing");
    let line = get("line-001");
    let splines = get("splines");
    let composites = get("composites");
    let streams = get("streams");

    offsets(&mut b, &drawing, &line);
    counts(&mut b, &drawing, &splines, &composites, &streams, &line);
    decompression(&mut b, &drawing);
    nesting(&mut b, &composites);
    knots(&mut b, &splines);
    weights(&mut b, &splines);
    domains(&mut b, &splines, &composites, &drawing);
    integrity(&mut b, &line);
    truncation(&mut b, &drawing);
    trailing(&mut b, &line);
    tags(&mut b, &drawing, &line, &splines, &streams);
    let gapped = get("gapped");
    corruption(&mut b, &drawing, &composites, &streams, &gapped);
    sweeps(&mut b);
    // A cut or a removed byte is refused for being wrong in whichever way the reader reached first, so
    // those cases allow any rule some other case pins for a refusal, and no other.
    let known: Vec<&'static str> = {
        let mut v: Vec<&'static str> = b
            .cases
            .iter()
            .filter(|c| c.requires == Requires::Parse)
            .filter_map(|c| if let Expect::Refuse(r) = &c.expect { Some(r.clone()) } else { None })
            .flatten()
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    for c in b.cases.iter_mut().filter(|c| c.family == Family::Truncation) {
        if let Expect::Refuse(r) = &mut c.expect {
            if r.is_empty() {
                *r = known.clone();
            }
        }
    }
    b.cases
}

fn offsets(b: &mut Builder, d: &At, line: &At) {
    use Family::Offsets as F;
    let n = d.len();
    let e0 = |name: &str| d.layout.blocks[0].entry + entry_field(name);
    let e1 = |name: &str| d.layout.blocks[1].entry + entry_field(name);
    let last = d.layout.blocks.last().expect("blocks").entry + entry_field("offset");
    b.refuse(
        F,
        "block_offset_plus_length_wraps",
        "drawing",
        vec![Op::u64(e0("offset"), u64::MAX - 7)],
        &["DIR-06"],
    );
    b.refuse(F, "block_offset_past_eof", "drawing", vec![Op::u64(e0("offset"), (n + 8) & !7)], &["DIR-06"]);
    b.refuse(
        F,
        "block_length_past_eof",
        "drawing",
        vec![Op::u32(e0("stored_length"), 0xFFFF_FF00), Op::u32(e0("uncompressed_length"), 0xFFFF_FF00)],
        &["DIR-06", "LIMIT-02"],
    );
    b.refuse(
        F,
        "overlapping_blocks",
        "drawing",
        vec![Op::u64(e1("offset"), d.layout.blocks[0].offset as u64)],
        &["DIR-08"],
    );
    b.refuse(
        F,
        "block_overlaps_manifest",
        "drawing",
        vec![Op::u64(e0("offset"), d.layout.manifest as u64)],
        &["DIR-11"],
    );
    b.refuse(
        F,
        "block_overlaps_directory",
        "drawing",
        vec![Op::u64(last, d.layout.directory as u64)],
        &["DIR-11", "DIR-06"],
    );
    b.refuse(
        F,
        "block_offset_not_aligned",
        "drawing",
        vec![Op::u64(e0("offset"), d.layout.blocks[0].offset as u64 + 1)],
        &["DIR-07"],
    );
    let ln = line.len();
    b.refuse(
        F,
        "header_manifest_offset_wraps",
        "line-001",
        vec![Op::u64(h::MANIFEST_OFFSET, u64::MAX)],
        &["PointerOutOfRange"],
    );
    b.refuse(
        F,
        "header_manifest_length_wraps",
        "line-001",
        vec![Op::u64(h::MANIFEST_LENGTH, u64::MAX - 8)],
        &["PointerOutOfRange", "ManifestTooLarge"],
    );
    b.refuse(
        F,
        "header_directory_table_past_eof",
        "line-001",
        vec![Op::u64(h::DIRECTORY_TABLE_OFFSET, ln + 8)],
        &["PointerOutOfRange"],
    );
    let ptr = line.layout.pointer_table;
    b.refuse(
        F,
        "manifest_directory_pointer_past_eof",
        "line-001",
        vec![Op::u64(ptr + dir::OFFSET, ln + 64)],
        &["PointerOutOfRange"],
    );
    b.refuse(
        F,
        "manifest_directory_pointer_wraps",
        "line-001",
        vec![Op::u64(ptr + dir::OFFSET, u64::MAX - 63)],
        &["PointerOutOfRange"],
    );
    b.refuse(
        F,
        "directory_ends_before_eof",
        "line-001",
        vec![Op::u64(ptr + dir::LENGTH, line.layout.directory_len as u64 - ENTRY_LEN as u64)],
        &["LAYOUT-03", "DIR-01", "PointerOutOfRange"],
    );
}

fn counts(b: &mut Builder, d: &At, s: &At, c: &At, st: &At, line: &At) {
    use Family::Counts as F;
    let lines = d.hot_field(LINE, "entity_count");
    b.refuse(
        F,
        "hot_entities_above_file_size",
        "drawing",
        vec![Op::u32(lines, 0x0FFF_FFFF)],
        &["HOT-01", "HOT-07", "HOT-10"],
    );
    b.refuse(
        F,
        "hot_entities_above_maximum",
        "drawing",
        vec![Op::u32(lines, 1 << 28)],
        &["HOT-09", "HOT-01"],
    );
    b.refuse(F, "hot_entities_u32_max", "drawing", vec![Op::u32(lines, u32::MAX)], &["HOT-09", "HOT-01"]);
    b.refuse(
        F,
        "hot_vertices_above_file_size",
        "drawing",
        vec![Op::u32(d.hot_field(POLYLINE, "vertex_count"), 0x0FFF_FFFF)],
        &["HOT-01", "HOT-10", "HOT-14"],
    );
    let records = s.block(2, 0).offset + field_at(RECORD_HEADER_FIELDS, "record_count");
    b.refuse(
        F,
        "records_above_file_size",
        "splines",
        vec![Op::u32(records, 0x0FFF_FFFF)],
        &["ARN-01", "ARN-07", "ARN-24"],
    );
    b.refuse(F, "records_above_maximum", "splines", vec![Op::u32(records, 1 << 28)], &["ARN-24", "ARN-01"]);
    let knots = s.arena(KNOTS);
    b.refuse(
        F,
        "arena_elements_u32_max",
        "splines",
        vec![Op::u32(knots.header("element_count"), u32::MAX)],
        &["ARN-01", "ARN-07", "ARN-11"],
    );
    b.refuse(
        F,
        "arena_entries_u32_max",
        "splines",
        vec![Op::u32(knots.header("entry_count"), u32::MAX)],
        &["ARN-01", "ARN-07"],
    );
    b.refuse(
        F,
        "control_points_u32_max",
        "splines",
        vec![Op::u32(s.run(CONTROL_POINTS, 0, "control_points").0 + 4, u32::MAX)],
        &["ARN-11", "ARN-13"],
    );
    b.refuse(
        F,
        "knots_u32_max",
        "splines",
        vec![Op::u32(s.run(KNOTS, 0, "knots").0 + 4, u32::MAX)],
        &["ARN-11", "ARN-13"],
    );
    b.refuse(
        F,
        "segments_above_arena",
        "composites",
        vec![Op::u32(c.run(SEGMENTS, 2, "segments").0 + 4, 65_537)],
        &["ARN-11", "ARN-24"],
    );
    b.refuse(
        F,
        "directory_entry_count_lies",
        "drawing",
        vec![Op::u32(d.layout.blocks[0].entry + entry_field("count"), u32::MAX)],
        &["CONSIST-01"],
    );
    b.refuse(
        F,
        "manifest_capabilities_u32_max",
        "line-001",
        vec![Op::u32(line.manifest(m::CAPABILITY_COUNT), u32::MAX)],
        &["Truncated"],
    );
    b.refuse(
        F,
        "manifest_directories_u32_max",
        "line-001",
        vec![Op::u32(line.manifest(m::DIRECTORY_COUNT), u32::MAX)],
        &["Truncated", "DirectoryTableMismatch", "BadDirectoryTableLength"],
    );
    b.refuse(
        F,
        "manifest_string_u32_max",
        "line-001",
        vec![Op::u32(line.manifest(m::PRODUCER_ID_LEN), u32::MAX)],
        &["StringTooLong"],
    );
    let set = st.block(4, 0).offset;
    b.refuse(F, "stream_count_above_maximum", "streams", vec![Op::u32(set + 8, 5_000)], &["STREAM-05"]);
    b.refuse(F, "stream_count_above_file_size", "streams", vec![Op::u32(set + 8, 4_000)], &["STREAM-04"]);
}

fn decompression(b: &mut Builder, d: &At) {
    use Family::Decompression as F;
    let entry = |i: usize, name: &str| d.layout.blocks[i].entry + entry_field(name);
    let stored = |i: usize| d.layout.blocks[i].len as u32;
    let limits = |name: &str, ops: Vec<Op>| (name.to_string(), ops);
    let cases = [
        limits("ratio_beyond_budget", vec![Op::u32(entry(0, "uncompressed_length"), stored(0) * 65)]),
        limits(
            "ratio_beyond_budget_declared_u32_max",
            vec![Op::u32(entry(0, "uncompressed_length"), u32::MAX)],
        ),
        limits(
            "ratio_beyond_budget_on_every_block",
            (0..d.layout.blocks.len())
                .map(|i| Op::u32(entry(i, "uncompressed_length"), stored(i) * 1000))
                .collect(),
        ),
    ];
    for (name, ops) in cases {
        b.add(F, Requires::Limits, &name, "drawing", ops, Expect::Refuse(vec!["LIMIT-01"]));
    }
}

fn nesting(b: &mut Builder, c: &At) {
    use Family::Nesting as F;
    // records by id: 10, 11, 20 (a), 21 (b), 22 (c), 23 (d), 24 (e); a's first segment is the first
    // element of its list
    let (a_run, b_run) = (c.run(SEGMENTS, 2, "segments"), c.run(SEGMENTS, 3, "segments"));
    let segs = c.arena(SEGMENTS);
    let first_of_a = segs.column(a_run.1.0, 4);
    assert_eq!(b_run.1.1, 2);
    b.refuse(F, "composite_names_itself", "composites", vec![Op::u32(first_of_a, 2)], &["ARN-21"]);
    b.refuse(F, "composites_name_each_other", "composites", vec![Op::u32(first_of_a, 3)], &["ARN-21"]);
    b.refuse(F, "chain_deeper_than_maximum", "composites", vec![Op::u32(first_of_a, 5)], &["ARN-22"]);
    b.refuse(F, "segment_names_missing_record", "composites", vec![Op::u32(first_of_a, 1000)], &["ARN-20"]);
    b.refuse(F, "segment_list_empty", "composites", vec![Op::u32(a_run.0 + 4, 0)], &["ARN-24"]);
}

fn knots(b: &mut Builder, s: &At) {
    use Family::Knots as F;
    let k = s.knot_values(0);
    assert_eq!(k.len(), 11, "the degree 2 curve with eight control points");
    // non-decreasing is the rule: put two interior knots in the wrong order
    b.refuse(
        F,
        "non_monotonic",
        "splines",
        vec![Op::f64(s.knot(0, 3), k[4]), Op::f64(s.knot(0, 4), k[3])],
        &["ARN-24"],
    );
    let knot_len = s.run(KNOTS, 0, "knots").0 + 4;
    b.refuse(F, "wrong_length_shorter", "splines", vec![Op::u32(knot_len, 10)], &["ARN-13"]);
    b.refuse(F, "wrong_length_longer", "splines", vec![Op::u32(knot_len, 12)], &["ARN-13"]);
    b.refuse(
        F,
        "control_point_count_changed",
        "splines",
        vec![Op::u32(s.run(CONTROL_POINTS, 0, "control_points").0 + 4, 7)],
        &["ARN-13"],
    );
    // interior multiplicity 5, where the largest a degree 2 curve allows is degree + 1 = 3
    b.refuse(
        F,
        "interior_multiplicity_above_degree_plus_one",
        "splines",
        (4..=7).map(|i| Op::f64(s.knot(0, i), k[3])).collect(),
        &["ARN-24"],
    );
}

fn weights(b: &mut Builder, s: &At) {
    use Family::Weights as F;
    let (entry, (start, len)) = s.run(WEIGHTS, 1, "weights");
    assert_eq!(len, 7, "the rational curve has seven weights");
    let w = |i: usize| s.arena(WEIGHTS).column(start + i, 8);
    b.refuse(F, "zero", "splines", vec![Op::f64(w(2), 0.0)], &["ARN-24"]);
    b.refuse(F, "negative", "splines", vec![Op::f64(w(2), -1.5)], &["ARN-24"]);
    b.refuse(F, "nan", "splines", vec![nan(w(2))], &["ARN-24"]);
    b.refuse(F, "infinite", "splines", vec![inf(w(2))], &["ARN-24"]);
    // positive and finite, but under the conditioning floor of NP-15
    b.refuse(
        F,
        "below_conditioning_floor",
        "splines",
        vec![Op::f64(w(2), f64::MIN_POSITIVE * 4.0)],
        &["ARN-24"],
    );
    b.refuse(
        F,
        "missing_on_rational",
        "splines",
        vec![Op::u32(s.record(1, "weights"), u32::MAX)],
        &["ARN-14", "ARN-18"],
    );
    b.refuse(
        F,
        "present_on_non_rational",
        "splines",
        vec![Op::u32(s.record(0, "weights"), 0)],
        &["ARN-14", "ARN-18"],
    );
    b.refuse(F, "count_short", "splines", vec![Op::u32(entry + 4, 6)], &["ARN-14"]);
    b.refuse(F, "count_long", "splines", vec![Op::u32(entry + 4, 8)], &["ARN-14", "ARN-11"]);
}

fn domains(b: &mut Builder, s: &At, c: &At, d: &At) {
    use Family::Domains as F;
    let k = s.knot_values(0);
    let (at, n) = s.knots(0);
    // the whole vector, reversed: the end of the domain is below its start
    let reversed: Vec<u8> = k.iter().rev().flat_map(|v| v.to_bits().to_le_bytes()).collect();
    b.refuse(F, "knots_inverted", "splines", vec![Op::Put { at, bytes: reversed }], &["ARN-24"]);
    // every knot equal: a domain of width zero
    let flat: Vec<u8> = (0..n).flat_map(|_| 0.0f64.to_bits().to_le_bytes()).collect();
    b.refuse(F, "knots_zero_length", "splines", vec![Op::Put { at, bytes: flat }], &["ARN-24"]);
    b.refuse(F, "knot_nan", "splines", vec![nan(s.knot(0, 5))], &["ARN-24"]);
    b.refuse(F, "knot_infinite", "splines", vec![inf(s.knot(0, 5))], &["ARN-24"]);
    let (_, (cp, _)) = s.run(CONTROL_POINTS, 0, "control_points");
    let cps = s.arena(CONTROL_POINTS);
    b.refuse(F, "control_point_nan", "splines", vec![nan(cps.column(cp + 1, 8))], &["ARN-24"]);
    b.refuse(F, "control_point_infinite", "splines", vec![inf(cps.second_column(cp + 1))], &["ARN-24"]);
    // above 2^40, the coordinate range of NP-02
    b.refuse(
        F,
        "control_point_beyond_coordinate_range",
        "splines",
        vec![Op::f64(cps.column(cp, 8), 10_000_000_000_000.0)],
        &["ARN-24"],
    );
    let (trim_entry, (trim_start, _)) = c.run(TRIMS, 6, "trims");
    b.refuse(F, "trim_nan", "composites", vec![nan(c.arena(TRIMS).column(trim_start + 1, 8))], &["ARN-24"]);
    b.refuse(F, "trim_count_not_twice_segments", "composites", vec![Op::u32(trim_entry + 4, 3)], &["ARN-23"]);
    let (_, _, lines) = d.hot_parts(LINE);
    b.refuse(F, "hot_coordinate_nan", "drawing", vec![nan(lines[0])], &["HOT-16"]);
    let (_, _, arcs) = d.hot_parts(CIRCLE_ARC);
    b.refuse(F, "hot_arc_sweep_infinite", "drawing", vec![inf(arcs[4])], &["HOT-16"]);
    let (_, _, polys) = d.hot_parts(POLYLINE);
    b.refuse(F, "hot_polyline_vertex_nan", "drawing", vec![nan(polys[0] + 8)], &["HOT-16"]);

    // What the hot parsers let through today, and a reader has to refuse (NP-01, NP-02, NP-06, NP-07).
    let later = |b: &mut Builder, name: &str, ops: Vec<Op>, rule: &'static str| {
        b.add(F, Requires::Semantics, name, "drawing", ops, Expect::Refuse(vec![rule]));
    };
    later(b, "hot_arc_radius_zero", vec![Op::f64(arcs[2], 0.0)], "NP-07");
    later(b, "hot_arc_radius_negative", vec![Op::f64(arcs[2], -1.0)], "NP-07");
    later(
        b,
        "hot_coordinate_beyond_coordinate_range",
        vec![Op::f64(lines[0], 10_000_000_000_000.0)],
        "NP-02",
    );
    later(b, "hot_coordinate_subnormal", vec![Op::f64(lines[0], f64::MIN_POSITIVE / 4.0)], "NP-01");
    let copy = |from: usize| d.bytes[from..from + 8].to_vec();
    later(
        b,
        "hot_line_zero_length",
        vec![
            Op::Put { at: lines[2], bytes: copy(lines[0]) },
            Op::Put { at: lines[3], bytes: copy(lines[1]) },
        ],
        "NP-06",
    );
    later(
        b,
        "hot_polyline_repeated_vertex",
        vec![Op::Put { at: polys[0] + 8, bytes: copy(polys[0]) }],
        "NP-06",
    );
}

fn integrity(b: &mut Builder, line: &At) {
    use Family::Integrity as F;
    let flags = line.manifest(m::FLAGS);
    let digest = line.manifest(m::ARTIFACT_DIGEST);
    let present =
        Op::u16(flags, u16::from_le_bytes([line.bytes[flags], line.bytes[flags + 1]]) | m::FLAG_DIGEST);
    let wrong = Op::Put { at: digest, bytes: vec![0xA5; 32] };
    // Whatever P4.6 defines the digest to cover, 32 bytes of 0xA5 are not it, and a digest that is
    // declared present has to be checked, not skipped.
    let pending = |b: &mut Builder, name: &str, ops: Vec<Op>, rule: &'static str| {
        b.add(F, Requires::Integrity, name, "line-001", ops, Expect::Refuse(vec![rule]));
    };
    pending(b, "artifact_digest_mismatch", vec![present.clone(), wrong], "INTEGRITY-01");
    pending(b, "artifact_digest_present_but_zero", vec![present], "INTEGRITY-01");
    // a checksum table declared over bytes that are a block's, not a table's
    let block = line.layout.blocks[0];
    pending(
        b,
        "block_checksum_table_declared_over_block_bytes",
        vec![
            Op::u64(line.manifest(m::BLOCK_CHECKSUMS_OFFSET), block.offset as u64),
            Op::u64(line.manifest(m::BLOCK_CHECKSUMS_LENGTH), 32),
        ],
        "INTEGRITY-02",
    );
}

fn truncation(b: &mut Builder, d: &At) {
    use Family::Truncation as F;
    let l = &d.layout;
    let first = l.blocks[0];
    let cuts = [
        ("at_zero", 0),
        ("in_magic", 5),
        ("in_header", 40),
        ("header_only", 64),
        ("in_manifest", l.manifest + l.manifest_len / 2),
        ("before_directory", l.directory),
        ("in_directory", l.directory + l.directory_len / 2),
        ("in_block_header", first.offset + 16),
        ("in_block_payload", first.offset + first.len - 8),
        ("last_byte", d.bytes.len() - 1),
    ];
    for (name, at) in cuts {
        // a cut of a valid file is refused for being short; which rule says so depends on what the
        // reader reached first
        b.add(
            F,
            Requires::Parse,
            name,
            "drawing",
            vec![Op::Truncate { len: at }],
            Expect::Refuse(Vec::new()),
        );
    }
}

fn trailing(b: &mut Builder, line: &At) {
    use Family::TrailingBytes as F;
    let junk: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(31) + 7) as u8).collect();
    for (name, bytes) in [
        ("one_zero_byte", vec![0u8]),
        ("eight_zero_bytes", vec![0u8; 8]),
        ("junk_4kib", junk),
        ("second_copy", line.bytes.to_vec()),
    ] {
        b.refuse(F, name, "line-001", vec![Op::Append { bytes }], &["LAYOUT-03"]);
    }
}

fn tags(b: &mut Builder, d: &At, line: &At, s: &At, st: &At) {
    use Family::UnknownTags as F;
    let e0 = |name: &str| d.layout.blocks[0].entry + entry_field(name);
    b.refuse(F, "unknown_required_block_kind", "drawing", vec![Op::u16(e0("kind"), 9)], &["DIR-04"]);
    // the component block is optional: a reader that does not know the kind skips it (P4.7)
    let comp = st.block(4, 0);
    b.add(
        F,
        Requires::Negotiation,
        "unknown_optional_block_kind_is_skipped",
        "streams",
        vec![Op::u16(comp.entry + entry_field("kind"), 9)],
        Expect::Accept,
    );
    b.refuse(F, "unknown_hot_kind", "drawing", vec![Op::u16(d.hot_field(LINE, "kind"), 9)], &["HOT-04"]);
    b.refuse(F, "unknown_record_kind", "splines", vec![Op::u8(s.record(0, "kind"), 9)], &["ARN-04"]);
    b.refuse(
        F,
        "unknown_payload_kind",
        "splines",
        vec![Op::u16(s.arena(KNOTS).header("payload_kind"), 9)],
        &["ARN-04"],
    );
    b.refuse(F, "unknown_codec_on_required_block", "drawing", vec![Op::u8(e0("codec"), 9)], &["DIR-05"]);
    b.refuse(
        F,
        "unknown_checksum_algorithm",
        "line-001",
        vec![Op::u8(h::BLOCK_CHECKSUM, 9)],
        &["UnknownChecksumAlgorithm"],
    );
    b.refuse(
        F,
        "unknown_length_unit",
        "line-001",
        vec![Op::u16(line.manifest(m::UNIT), 99)],
        &["UnknownLengthUnit"],
    );
    b.refuse(
        F,
        "unknown_manifest_flag",
        "line-001",
        vec![Op::u16(line.manifest(m::FLAGS), 0x8000)],
        &["InvalidFlags"],
    );
    let flags = d.bytes[e0("flags")];
    b.refuse(F, "unknown_directory_flag", "drawing", vec![Op::u8(e0("flags"), flags | 0x80)], &["DIR-03"]);
    let rec_flags = u16::from_le_bytes([s.bytes[s.record(0, "flags")], s.bytes[s.record(0, "flags") + 1]]);
    b.refuse(
        F,
        "unknown_record_flag",
        "splines",
        vec![Op::u16(s.record(0, "flags"), rec_flags | 0x8000)],
        &["ARN-17"],
    );
    b.refuse(
        F,
        "periodic_record_flag",
        "splines",
        vec![Op::u16(s.record(0, "flags"), rec_flags | 4)],
        &["ARN-16"],
    );
    b.refuse(
        F,
        "unknown_capability_requirement",
        "line-001",
        vec![Op::u8(line.manifest(pvec_container::Manifest::FIXED_LEN + cap::REQUIREMENT), 9)],
        &["InvalidRequirement"],
    );
    b.refuse(
        F,
        "future_container_major_version",
        "line-001",
        vec![Op::u16(h::MAJOR, 2)],
        &["UnsupportedVersion"],
    );
    b.refuse(F, "bad_magic", "line-001", vec![Op::u8(h::MAGIC, 0x90)], &["BadMagic"]);
    b.refuse(
        F,
        "big_endian_marker",
        "line-001",
        vec![Op::Put { at: h::BYTE_ORDER, bytes: vec![0x01, 0x02, 0x03, 0x04] }],
        &["UnsupportedByteOrder"],
    );
    b.refuse(F, "future_hot_version", "drawing", vec![Op::u16(d.hot_field(LINE, "version"), 2)], &["HOT-03"]);
    b.refuse(
        F,
        "future_arena_version",
        "splines",
        vec![Op::u16(s.arena(KNOTS).header("version"), 2)],
        &["ARN-03"],
    );
    b.refuse(
        F,
        "unknown_stream_set_version",
        "streams",
        vec![Op::u16(st.block(4, 0).offset + 4, 2)],
        &["STREAM-02"],
    );
}

fn corruption(b: &mut Builder, d: &At, c: &At, st: &At, gapped: &At) {
    use Family::Corruption as F;
    let e = |i: usize, name: &str| d.layout.blocks[i].entry + entry_field(name);
    b.refuse(
        F,
        "hot_block_length_lies",
        "drawing",
        vec![Op::u32(d.hot_field(LINE, "block_length"), d.layout.blocks[0].len as u32 + 8)],
        &["HOT-07", "HOT-01"],
    );
    b.refuse(
        F,
        "arena_block_length_lies",
        "composites",
        vec![Op::u32(c.arena(SEGMENTS).header("block_length"), c.block(3, SEGMENTS).len as u32 + 8)],
        &["ARN-07", "ARN-01"],
    );
    b.refuse(
        F,
        "records_block_length_lies",
        "composites",
        vec![Op::u32(
            c.block(2, 0).offset + field_at(RECORD_HEADER_FIELDS, "block_length"),
            c.block(2, 0).len as u32 - 8,
        )],
        &["ARN-07", "ARN-01"],
    );
    b.refuse(
        F,
        "hot_reserved_nonzero",
        "drawing",
        vec![Op::u8(d.hot_field(LINE, "reserved"), 1)],
        &["HOT-05"],
    );
    b.refuse(F, "directory_reserved_nonzero", "drawing", vec![Op::u8(e(0, "reserved"), 1)], &["DIR-02"]);
    b.refuse(F, "header_reserved_nonzero", "drawing", vec![Op::u8(h::RESERVED_C, 1)], &["NonZeroReserved"]);
    let (ids, _, _) = d.hot_parts(LINE);
    b.refuse(
        F,
        "hot_ids_not_ascending",
        "drawing",
        vec![Op::u32(ids, u32_at(d.bytes, ids + 4)), Op::u32(ids + 4, u32_at(d.bytes, ids))],
        &["HOT-12"],
    );
    let (r0, r1) = (c.record(0, "id"), c.record(1, "id"));
    b.refuse(
        F,
        "record_ids_not_ascending",
        "composites",
        vec![Op::u32(r0, u32_at(c.bytes, r1)), Op::u32(r1, u32_at(c.bytes, r0))],
        &["ARN-19"],
    );
    // the circle's id is also a line's: one entity in two blocks
    let (circle_ids, _, _) = d.hot_parts(CIRCLE_ARC);
    b.refuse(
        F,
        "entity_in_two_blocks",
        "drawing",
        vec![Op::u32(circle_ids, u32_at(d.bytes, ids))],
        &["CONSIST-03"],
    );
    // two directory entries swapped whole: the second block now starts before the first ends
    let (e0, e1) = (d.layout.blocks[0].entry, d.layout.blocks[1].entry);
    let (a, bb) = (d.bytes[e0..e0 + ENTRY_LEN].to_vec(), d.bytes[e1..e1 + ENTRY_LEN].to_vec());
    b.refuse(
        F,
        "directory_entries_swapped",
        "drawing",
        vec![Op::Put { at: e0, bytes: bb }, Op::Put { at: e1, bytes: a }],
        &["DIR-08"],
    );
    // only the kind and geometry type swapped, so the offsets still ascend: the blocks are out of
    // canonical order
    let key = |at: usize| d.bytes[at + entry_field("kind")..at + entry_field("kind") + 4].to_vec();
    let (k0, k1) = (key(e0), key(e1));
    b.refuse(
        F,
        "directory_entries_out_of_order",
        "drawing",
        vec![
            Op::Put { at: e0 + entry_field("kind"), bytes: k1 },
            Op::Put { at: e1 + entry_field("kind"), bytes: k0 },
        ],
        &["LAYOUT-04"],
    );
    let (_, poly_runs, _) = d.hot_parts(POLYLINE);
    b.refuse(
        F,
        "polyline_run_offsets_decrease",
        "drawing",
        vec![Op::u32(poly_runs + 4, 9)],
        &["HOT-13", "HOT-14"],
    );
    // the layout rules of the reference
    b.refuse(
        F,
        "header_length_below_fixed_header",
        "drawing",
        vec![Op::u16(h::HEADER_LEN, 32)],
        &["BadHeaderLength"],
    );
    // the manifest's one directory pointer says it points at a spatial directory, not the block directory
    b.refuse(
        F,
        "directory_pointer_of_another_kind",
        "drawing",
        vec![Op::u32(d.layout.pointer_table + dir::KIND, 3)],
        &["LAYOUT-03"],
    );
    b.refuse(
        F,
        "manifest_not_after_header",
        "drawing",
        vec![Op::u64(h::MANIFEST_OFFSET, 72)],
        &["LAYOUT-01"],
    );
    let caps = d.layout.manifest + pvec_container::Manifest::FIXED_LEN;
    let (id0, id1) = (u32_at(d.bytes, caps + cap::ID), u32_at(d.bytes, caps + cap::SIZE + cap::ID));
    assert!(id0 < id1, "the base declares its capabilities in ascending order");
    b.refuse(
        F,
        "capabilities_out_of_order",
        "drawing",
        vec![Op::u32(caps + cap::ID, id1), Op::u32(caps + cap::SIZE + cap::ID, id0)],
        &["LAYOUT-02"],
    );
    let records = d.layout.blocks.iter().find(|x| x.kind == 2).expect("a records block");
    b.refuse(
        F,
        "record_count_disagrees_with_directory",
        "drawing",
        vec![Op::u32(
            records.entry + entry_field("count"),
            u32_at(d.bytes, records.entry + entry_field("count")) - 1,
        )],
        &["CONSIST-02"],
    );
    // the last arena is filed under a payload kind nobody asks for, so the advanced blocks are one short
    let trims = d.block(3, TRIMS);
    b.refuse(
        F,
        "arena_block_missing",
        "drawing",
        vec![Op::u16(trims.entry + entry_field("subtype"), 6)],
        &["CONSIST-04"],
    );
    b.refuse(
        F,
        "directory_empty",
        "drawing",
        vec![Op::Truncate { len: d.layout.directory }, Op::u64(d.layout.pointer_table + dir::LENGTH, 0)],
        &["LAYOUT-06"],
    );
    // a legal gap of zero bytes between blocks and before the directory, with a byte of it set
    let g = &gapped.layout;
    b.refuse(
        F,
        "padding_nonzero_between_blocks",
        "gapped",
        vec![Op::u8(g.blocks[1].offset - 1, 1)],
        &["LAYOUT-08"],
    );
    b.refuse(
        F,
        "padding_nonzero_before_directory",
        "gapped",
        vec![Op::u8(g.directory - 1, 1)],
        &["LAYOUT-08"],
    );
    let set = st.block(4, 0).offset;
    b.refuse(F, "stream_length_lies", "streams", vec![Op::u32(set + 16 + 12, 0x00FF_FFFF)], &["STREAM-04"]);
    b.refuse(F, "stream_set_trailing_bytes", "streams", vec![Op::u32(set + 8, 0)], &["STREAM-07"]);
    b.refuse(F, "stream_reserved_nonzero", "streams", vec![Op::u16(set + 16 + 10, 1)], &["STREAM-03"]);
}

/// Every prefix of every base, and every base with one byte removed. Two sweeps that land on the same
/// bytes (removing the last byte is a cut) or on bytes a named case already has are one case.
fn sweeps(b: &mut Builder) {
    let digest = |v: &[u8]| {
        let mut h = DefaultHasher::new();
        v.hash(&mut h);
        h.finish()
    };
    let mut seen: HashSet<u64> =
        b.cases.iter().map(|c| digest(&apply(&b.bases[c.base].bytes, &c.ops))).collect();
    let mut added = Vec::new();
    for (base, bs) in b.bases.iter().enumerate() {
        for len in 0..bs.bytes.len() {
            if seen.insert(digest(&bs.bytes[..len])) {
                added.push(Case {
                    name: format!("truncation/prefix/{}/{len}", bs.name),
                    family: Family::Truncation,
                    requires: Requires::Parse,
                    base,
                    ops: vec![Op::Truncate { len }],
                    expect: Expect::Refuse(Vec::new()),
                });
            }
        }
    }
    for (base, bs) in b.bases.iter().enumerate() {
        for at in 0..bs.bytes.len() {
            let mut v = bs.bytes.clone();
            v.remove(at);
            if seen.insert(digest(&v)) {
                added.push(Case {
                    name: format!("truncation/remove_byte/{}/{at}", bs.name),
                    family: Family::Truncation,
                    requires: Requires::Parse,
                    base,
                    ops: vec![Op::Remove { at, len: 1 }],
                    expect: Expect::Refuse(Vec::new()),
                });
            }
        }
    }
    b.cases.extend(added);
}
