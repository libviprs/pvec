//! Resource limits for untrusted `.pvec` files (issue P4.8).
//!
//! A file can be uploaded by anyone and decoded inside a browser tab, so every count and every offset in
//! it is attacker-controlled. [`Limits`] is the one configurable set of ceilings, and it is enforced in
//! the only place that matters: before an allocation and before work. A ceiling that is exceeded is a
//! [`LimitExceeded`], a distinct error that says which limit, what the file declared and what the
//! ceiling was. It is never a panic and never a silent clamp.
//!
//! The geometric ceilings cite the maxima of NP-18 (`pvec-numeric`), they do not restate them: the
//! defaults are those constants, and [`Limits::validate`] refuses a ceiling above one, since the block
//! parsers refuse anything past NP-18 whatever the configuration says. What a configuration can do is
//! lower them (the browser preset does) or, for the limits that are not NP-18's, raise them.
//!
//! [`Accountant`] is the allocation accountant the reader threads through every buffer it makes. It
//! counts live bytes, not one allocation at a time, so many individually legal blocks cannot add up to
//! more than the budget, and it counts decode work cumulatively. `docs/SECURITY_LIMITS.md` is rendered
//! from [`LIMIT_TABLE`] and the two presets.

use core::fmt;
use pvec_components::{MAX_CALIBRATION_BYTES, MAX_RASTERS};
use pvec_numeric::{MAX_CONTROL_POINTS, MAX_ENTITIES, MAX_KNOTS, MAX_NESTING_DEPTH};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Which limit was exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LimitKind {
    /// A block declares more decoded bytes than its stored bytes times the expansion ratio.
    ExpansionRatio,
    /// A block is stored or decodes to more bytes than the block ceiling.
    BlockBytes,
    /// The live buffers of the reader would add up to more than the allocation budget.
    AllocationBudget,
    /// The decode work done so far would pass the work budget.
    DecodeWork,
    /// The artifact is longer than the artifact ceiling.
    ArtifactBytes,
    /// The block directory has more entries than the ceiling.
    Blocks,
    /// The directories declare more entities than the artifact ceiling.
    EntitiesPerArtifact,
    /// A block declares more entities than the block ceiling.
    EntitiesPerBlock,
    /// A curve has more control points than the ceiling.
    ControlPoints,
    /// A curve has more knots than the ceiling.
    Knots,
    /// Composite curves are nested deeper than the ceiling.
    NestingDepth,
    /// A descriptor's offset plus length passes the end of the artifact, or overflows. The ceiling is
    /// the artifact's own length, so there is no field for it.
    Extent,
    /// An artifact declares more raster placements than the ceiling (ADR 0018).
    Rasters,
    /// An artifact declares more raster tiles than the ceiling.
    RasterTiles,
    /// A raster tile is stored as more bytes than the placement's formula allows.
    TileBytes,
    /// A plan would hand the host more decoded raster bytes at once than the ceiling.
    PlanDecodedBytes,
    /// The calibration block (kind 11) is stored as, or decodes to, more bytes than the ceiling (ADR 0019).
    CalibrationBytes,
}

impl LimitKind {
    /// Every limit, in rule order.
    pub const ALL: [LimitKind; 17] = [
        LimitKind::ExpansionRatio,
        LimitKind::BlockBytes,
        LimitKind::AllocationBudget,
        LimitKind::DecodeWork,
        LimitKind::ArtifactBytes,
        LimitKind::Blocks,
        LimitKind::EntitiesPerArtifact,
        LimitKind::EntitiesPerBlock,
        LimitKind::ControlPoints,
        LimitKind::Knots,
        LimitKind::NestingDepth,
        LimitKind::Extent,
        LimitKind::Rasters,
        LimitKind::RasterTiles,
        LimitKind::TileBytes,
        LimitKind::PlanDecodedBytes,
        LimitKind::CalibrationBytes,
    ];

    /// The rule identifier of `docs/SECURITY_LIMITS.md`.
    pub fn rule(self) -> &'static str {
        match self {
            LimitKind::ExpansionRatio => "LIMIT-01",
            LimitKind::BlockBytes => "LIMIT-02",
            LimitKind::AllocationBudget => "LIMIT-03",
            LimitKind::DecodeWork => "LIMIT-04",
            LimitKind::ArtifactBytes => "LIMIT-05",
            LimitKind::Blocks => "LIMIT-06",
            LimitKind::EntitiesPerArtifact => "LIMIT-07",
            LimitKind::EntitiesPerBlock => "LIMIT-08",
            LimitKind::ControlPoints => "LIMIT-09",
            LimitKind::Knots => "LIMIT-10",
            LimitKind::NestingDepth => "LIMIT-11",
            LimitKind::Extent => "LIMIT-12",
            LimitKind::Rasters => "LIMIT-13",
            LimitKind::RasterTiles => "LIMIT-14",
            LimitKind::TileBytes => "LIMIT-15",
            LimitKind::PlanDecodedBytes => "LIMIT-16",
            LimitKind::CalibrationBytes => "LIMIT-17",
        }
    }

    /// The name of the field of [`Limits`] that sets it.
    pub fn field(self) -> &'static str {
        match self {
            LimitKind::ExpansionRatio => "max_expansion_ratio",
            LimitKind::BlockBytes => "max_block_bytes",
            LimitKind::AllocationBudget => "max_alloc_bytes",
            LimitKind::DecodeWork => "max_decode_work",
            LimitKind::ArtifactBytes => "max_artifact_bytes",
            LimitKind::Blocks => "max_blocks",
            LimitKind::EntitiesPerArtifact => "max_entities_per_artifact",
            LimitKind::EntitiesPerBlock => "max_entities_per_block",
            LimitKind::ControlPoints => "max_control_points",
            LimitKind::Knots => "max_knots",
            LimitKind::NestingDepth => "max_nesting_depth",
            LimitKind::Extent => "the artifact length",
            LimitKind::Rasters => "max_rasters",
            LimitKind::RasterTiles => "max_raster_tiles",
            LimitKind::TileBytes => "max_tile_bytes",
            LimitKind::PlanDecodedBytes => "max_plan_decoded_bytes",
            LimitKind::CalibrationBytes => "max_calibration_bytes",
        }
    }
}

/// A limit that a file exceeded: which one, what the file declared and the ceiling it was held to.
///
/// `declared` is what the file asked for (a decoded length, a count), or for the budgets the total the
/// request would have brought the reader to. For a nesting depth it is the first level over the
/// ceiling, since the walk stops there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitExceeded {
    /// Which limit.
    pub limit: LimitKind,
    /// What the file declared.
    pub declared: u64,
    /// The ceiling.
    pub ceiling: u64,
}

impl fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} is {} against a ceiling of {}",
            self.limit.rule(),
            self.limit.field(),
            self.declared,
            self.ceiling
        )
    }
}

impl std::error::Error for LimitExceeded {}

/// Why a [`Limits`] value is not usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitsError {
    /// The limit.
    pub limit: LimitKind,
    /// The value that was given.
    pub given: u64,
    /// The lowest value that makes sense for it.
    pub format_minimum: u64,
    /// The highest value the format allows for it.
    pub format_maximum: u64,
}

impl fmt::Display for LimitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.given < self.format_minimum {
            write!(
                f,
                "{} is {}, below {}: the outermost curve counts as a level, so a ceiling under it refuses every file",
                self.limit.field(),
                self.given,
                self.format_minimum
            )
        } else {
            write!(
                f,
                "{} is {}, above the {} the format allows, which the block parsers refuse whatever this says",
                self.limit.field(),
                self.given,
                self.format_maximum
            )
        }
    }
}

impl std::error::Error for LimitsError {}

/// The ceilings a reader holds a file to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The longest artifact, in bytes.
    pub max_artifact_bytes: u64,
    /// The most bytes one block may be stored as, and the most it may decode to.
    pub max_block_bytes: u32,
    /// The most blocks, which bounds the directory.
    pub max_blocks: u32,
    /// The most a block may decode to, as a multiple of its stored length.
    pub max_expansion_ratio: u32,
    /// The most entities (hot entities and advanced records) one artifact may declare.
    pub max_entities_per_artifact: u64,
    /// The most entities one block may declare.
    pub max_entities_per_block: u32,
    /// The most control points one curve may have. At most NP-18's maximum.
    pub max_control_points: u32,
    /// The most knots one curve may have. At most NP-18's maximum.
    pub max_knots: u32,
    /// The deepest composite nesting, the outermost curve counting as 1. At most NP-18's maximum.
    pub max_nesting_depth: u32,
    /// The most bytes of live decoded buffers (blocks in the cache and blocks a caller still holds).
    pub max_alloc_bytes: u64,
    /// The most decode work over the life of a reader, in bytes stored plus bytes decoded.
    pub max_decode_work: u64,
    /// The most raster placements one artifact may declare (ADR 0018, proposed).
    pub max_rasters: u32,
    /// The most raster tiles one artifact may declare, over every raster (proposed).
    pub max_raster_tiles: u32,
    /// The header allowance added to `2 * 4 * tile_edge^2` to give the most bytes one raster tile may be stored
    /// as: the formula is a function of the placement, so this field is only its constant term (proposed).
    pub max_tile_bytes: u32,
    /// The most decoded raster bytes one plan may hand the host at once (proposed).
    pub max_plan_decoded_bytes: u64,
    /// The most bytes the calibration block may be stored as or decode to (ADR 0019).
    pub max_calibration_bytes: u32,
}

impl Limits {
    /// The defaults for a native reader.
    pub const DEFAULT: Limits = Limits {
        max_artifact_bytes: 1 << 36,
        max_block_bytes: 1 << 28,
        max_blocks: 1 << 20,
        max_expansion_ratio: 64,
        max_entities_per_artifact: MAX_ENTITIES as u64,
        max_entities_per_block: MAX_ENTITIES as u32,
        max_control_points: MAX_CONTROL_POINTS as u32,
        max_knots: MAX_KNOTS as u32,
        max_nesting_depth: MAX_NESTING_DEPTH as u32,
        max_alloc_bytes: 1 << 31,
        max_decode_work: 1 << 40,
        max_rasters: MAX_RASTERS,
        max_raster_tiles: 1 << 20,
        max_tile_bytes: 1 << 16,
        max_plan_decoded_bytes: 1 << 28,
        max_calibration_bytes: MAX_CALIBRATION_BYTES,
    };

    /// The stricter preset for a browser tab, where memory is a few hundred megabytes and a hostile file
    /// is a tab that stops responding.
    pub const BROWSER: Limits = Limits {
        max_artifact_bytes: 1 << 32,
        max_block_bytes: 1 << 26,
        max_blocks: 1 << 17,
        max_expansion_ratio: 32,
        max_entities_per_artifact: 1 << 24,
        max_entities_per_block: 1 << 20,
        max_control_points: 1 << 14,
        max_knots: (1 << 14) + 17,
        max_nesting_depth: 3,
        max_alloc_bytes: 1 << 28,
        max_decode_work: 1 << 34,
        max_rasters: MAX_RASTERS,
        max_raster_tiles: 1 << 17,
        max_tile_bytes: 1 << 16,
        max_plan_decoded_bytes: 1 << 28,
        max_calibration_bytes: MAX_CALIBRATION_BYTES,
    };

    /// The value of one limit, widened.
    pub fn get(&self, kind: LimitKind) -> u64 {
        match kind {
            LimitKind::ExpansionRatio => u64::from(self.max_expansion_ratio),
            LimitKind::BlockBytes => u64::from(self.max_block_bytes),
            LimitKind::AllocationBudget => self.max_alloc_bytes,
            LimitKind::DecodeWork => self.max_decode_work,
            LimitKind::ArtifactBytes => self.max_artifact_bytes,
            LimitKind::Blocks => u64::from(self.max_blocks),
            LimitKind::EntitiesPerArtifact => self.max_entities_per_artifact,
            LimitKind::EntitiesPerBlock => u64::from(self.max_entities_per_block),
            LimitKind::ControlPoints => u64::from(self.max_control_points),
            LimitKind::Knots => u64::from(self.max_knots),
            LimitKind::NestingDepth => u64::from(self.max_nesting_depth),
            LimitKind::Extent => self.max_artifact_bytes,
            LimitKind::Rasters => u64::from(self.max_rasters),
            LimitKind::RasterTiles => u64::from(self.max_raster_tiles),
            LimitKind::TileBytes => u64::from(self.max_tile_bytes),
            LimitKind::PlanDecodedBytes => self.max_plan_decoded_bytes,
            LimitKind::CalibrationBytes => u64::from(self.max_calibration_bytes),
        }
    }

    /// Checks that no ceiling that cites NP-18 is above it, and that the nesting ceiling is at least 1. A
    /// ceiling above the format's maximum would promise a file the parsers refuse, so it is an error and
    /// not a quiet clamp.
    pub fn validate(&self) -> Result<(), LimitsError> {
        for (limit, given, format_minimum, format_maximum) in [
            (LimitKind::EntitiesPerArtifact, self.max_entities_per_artifact, 0, MAX_ENTITIES as u64),
            (LimitKind::EntitiesPerBlock, u64::from(self.max_entities_per_block), 0, MAX_ENTITIES as u64),
            (LimitKind::ControlPoints, u64::from(self.max_control_points), 0, MAX_CONTROL_POINTS as u64),
            (LimitKind::Knots, u64::from(self.max_knots), 0, MAX_KNOTS as u64),
            (LimitKind::NestingDepth, u64::from(self.max_nesting_depth), 1, MAX_NESTING_DEPTH as u64),
        ] {
            if given > format_maximum || given < format_minimum {
                return Err(LimitsError { limit, given, format_minimum, format_maximum });
            }
        }
        Ok(())
    }

    /// Refuses `declared` when it is above the ceiling of `kind`.
    pub fn check(&self, kind: LimitKind, declared: u64) -> Result<(), LimitExceeded> {
        let ceiling = self.get(kind);
        if declared > ceiling { Err(LimitExceeded { limit: kind, declared, ceiling }) } else { Ok(()) }
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}

/// One row of `docs/SECURITY_LIMITS.md`: what a limit protects and what the reader does at the ceiling.
#[derive(Clone, Copy, Debug)]
pub struct LimitRow {
    /// The limit.
    pub kind: LimitKind,
    /// Where it comes from: NP-18, or the reason for the value.
    pub rationale: &'static str,
    /// Where it is enforced.
    pub enforced: &'static str,
}

/// Every limit with its rationale and enforcement point.
pub const LIMIT_TABLE: &[LimitRow] = &[
    LimitRow {
        kind: LimitKind::ExpansionRatio,
        rationale: "A deflate stream can expand about 1000 times and an LZ4 block about 255 times, so an unbounded ratio is a decompression bomb. Real blocks of coordinates compress 2 to 4 times, so 64 leaves a wide margin.",
        enforced: "the block directory, from the declared lengths, before anything is fetched or reserved",
    },
    LimitRow {
        kind: LimitKind::BlockBytes,
        rationale: "A block is read and decoded whole, so its size is the largest single buffer. The directory carries 32-bit lengths; this is far below that.",
        enforced: "the block directory, from the stored and the declared decoded length, before the block is fetched",
    },
    LimitRow {
        kind: LimitKind::AllocationBudget,
        rationale: "The sum of the live decoded buffers, so many blocks that are each legal cannot add up to the machine's memory.",
        enforced: "the allocation accountant, before every decoded buffer is created; unreferenced cache entries are evicted first",
    },
    LimitRow {
        kind: LimitKind::DecodeWork,
        rationale: "Time, not memory: a file that makes the reader decode the same large blocks over and over would stall a tab without allocating more than the budget.",
        enforced: "the allocation accountant, before every block or stretch of padding is fetched",
    },
    LimitRow {
        kind: LimitKind::ArtifactBytes,
        rationale: "The length of the artifact as the source reports it. Offsets in the file are held to it, so this bounds every range the reader will ask for.",
        enforced: "opening, before the first range is requested",
    },
    LimitRow {
        kind: LimitKind::Blocks,
        rationale: "The directory is fetched whole, at 64 bytes a block, so this bounds what a cold open transfers and allocates (ADR 0008 measured 0.17% of the file at 512 entities per block).",
        enforced: "opening, from the directory pointer, before the directory is fetched",
    },
    LimitRow {
        kind: LimitKind::EntitiesPerArtifact,
        rationale: "Cites NP-18's maximum of entities in one file (2^28). The identity checks of a full validation hold one id per entity.",
        enforced: "opening, from the counts in the directory, and again when the whole file is validated",
    },
    LimitRow {
        kind: LimitKind::EntitiesPerBlock,
        rationale: "Cites NP-18's maximum of entities in one file, since one block may hold them all.",
        enforced: "the directory count, and the count in the block's own header before the block is parsed",
    },
    LimitRow {
        kind: LimitKind::ControlPoints,
        rationale: "Cites NP-18's maximum of control points on one curve (2^16).",
        enforced: "reading a curve, from the offset-table entry, before anything is copied",
    },
    LimitRow {
        kind: LimitKind::Knots,
        rationale: "Cites NP-18's maximum of knots on one curve (control points plus degree plus one at the maxima).",
        enforced: "reading a curve, from the offset-table entry, before anything is copied",
    },
    LimitRow {
        kind: LimitKind::NestingDepth,
        rationale: "Cites NP-18's maximum composite nesting depth (4, the outermost curve counting as 1). A walk deeper than this is refused before the segments are resolved, so the recursion is bounded by the ceiling.",
        enforced: "the nesting walk over the record block, before any segment is resolved",
    },
    LimitRow {
        kind: LimitKind::Extent,
        rationale: "Every offset and length in the file is attacker-controlled, and a range past the end of the artifact (or one whose end wraps around 2^64) is never fetched. The ceiling is the length of the artifact itself, so it has no configuration of its own and no browser value.",
        enforced: "the block directory, from the raw entries, before the table is parsed or any block fetched",
    },
    LimitRow {
        kind: LimitKind::Rasters,
        rationale: "Proposed (ADR 0018): 4,096 placements. Each raster has an index block of 64 bytes in the directory, so the ceiling is 256 KiB of directory, about 0.5% of a 50 MB file. Issue 220 measured the directory cost at 432 bytes per raster (one index block and five chunk blocks at 72 bytes each, native x64), not 64, so 4,096 rasters would be about 1.8 MB of directory, 3.5% of a 50 MB file and past that 0.5% line. The ceiling stays proposed for that reason, with an owner to decide whether it or the line moves; a drawing has a handful of rasters, not thousands.",
        enforced: "the raster stream, when the placements are read, before the table is allocated; and when the stream is written",
    },
    LimitRow {
        kind: LimitKind::RasterTiles,
        rationale: "Proposed (ADR 0018): 1,048,576 tiles per artifact and 131,072 in the browser. An index entry is about 24 bytes, so that is 24 MiB and 3 MiB of index at the ceilings, inside LIMIT-02; the numbers borrow LIMIT-06's. Measured by issue 220 (native x64, `docs/BENCHMARKS.md`): 24.44 bytes per tile, so 25.6 MB at the default ceiling and 3.2 MB in the browser. Kept.",
        enforced: "the raster index block (issue 218), from the counts in its header, before the tile table is allocated",
    },
    LimitRow {
        kind: LimitKind::TileBytes,
        rationale: "Proposed (ADR 0018): one tile is stored as at most `2 * 4 * tile_edge^2` plus this field's allowance of 65,536 bytes (589,824 at an edge of 256, 2,162,688 at 512, 8,454,144 at 1024). An uncompressed RGBA PNG of the tile is `4 * edge^2` plus framing, so twice that admits every honest encoding and refuses a progressive JPEG whose only purpose is decode time. Proposed still: it needs the stored tile sizes of real scans and a browser decoder, which issue 220 does not have.",
        enforced: "the tile read (issue 219), from the length in the raster index, before the range is requested",
    },
    LimitRow {
        kind: LimitKind::PlanDecodedBytes,
        rationale: "Proposed (ADR 0018): the decoded bytes a plan may hand the host at once, 256 MiB (the browser LIMIT-03): 256 tiles of 512 px, 1,024 of 256, 64 of 1024. The host decoder is outside the accountant, so the plan is where the bound lives. The plan enforces it (issue 220, `plan_raster`, tested at 256 tiles of 512 px, and a coarser level is chosen until it fits); proposed still as a number, because what the decoder and the textures cost in a browser is libviprs/maplibre-pvec#102's measurement.",
        enforced: "the raster plan (issue 220), before the first range of the plan is requested",
    },
    LimitRow {
        kind: LimitKind::CalibrationBytes,
        rationale: "ADR 0019: the calibration block is read at open, so it gets a ceiling far below LIMIT-02. 64 KiB is above the 49,894 bytes of the largest block the CAL rules allow (1,024 control points, a 16 KiB definition and the longest strings), so it never refuses a valid block; it is a field so a host can lower it.",
        enforced: "the reader, from the kind 11 directory entry's stored and decoded lengths, before the block is fetched; `decode_calibration` holds its input to the constant too",
    },
];

/// Counts the bytes of live buffers against an allocation budget and the decode work against a work
/// budget. Clones share the counters.
#[derive(Clone, Debug)]
pub struct Accountant {
    inner: Arc<Counters>,
}

#[derive(Debug)]
struct Counters {
    live: AtomicU64,
    peak: AtomicU64,
    work: AtomicU64,
    max_alloc: u64,
    max_work: u64,
}

/// Bytes held against an [`Accountant`]. Dropping it gives them back.
#[derive(Debug)]
pub struct Reservation {
    counters: Arc<Counters>,
    bytes: u64,
}

impl Reservation {
    /// How many bytes it holds.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.counters.live.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl Accountant {
    /// An accountant for the budgets of `limits`.
    pub fn new(limits: &Limits) -> Accountant {
        Accountant {
            inner: Arc::new(Counters {
                live: AtomicU64::new(0),
                peak: AtomicU64::new(0),
                work: AtomicU64::new(0),
                max_alloc: limits.max_alloc_bytes,
                max_work: limits.max_decode_work,
            }),
        }
    }

    /// Reserves `bytes` before a buffer of that size is made. Refused, with nothing reserved, when the
    /// live total would pass the budget.
    pub fn reserve(&self, bytes: u64) -> Result<Reservation, LimitExceeded> {
        let c = &self.inner;
        let mut live = c.live.load(Ordering::Acquire);
        loop {
            let total = live.saturating_add(bytes);
            if total > c.max_alloc {
                return Err(LimitExceeded {
                    limit: LimitKind::AllocationBudget,
                    declared: total,
                    ceiling: c.max_alloc,
                });
            }
            match c.live.compare_exchange_weak(live, total, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    c.peak.fetch_max(total, Ordering::AcqRel);
                    return Ok(Reservation { counters: Arc::clone(&self.inner), bytes });
                }
                Err(now) => live = now,
            }
        }
    }

    /// Adds `units` of decode work. Refused when the total would pass the work budget.
    pub fn charge_work(&self, units: u64) -> Result<(), LimitExceeded> {
        let c = &self.inner;
        let mut done = c.work.load(Ordering::Acquire);
        loop {
            let total = done.saturating_add(units);
            if total > c.max_work {
                return Err(LimitExceeded {
                    limit: LimitKind::DecodeWork,
                    declared: total,
                    ceiling: c.max_work,
                });
            }
            match c.work.compare_exchange_weak(done, total, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Ok(()),
                Err(now) => done = now,
            }
        }
    }

    /// Bytes reserved right now.
    pub fn live(&self) -> u64 {
        self.inner.live.load(Ordering::Acquire)
    }

    /// The most bytes that were ever reserved at once.
    pub fn peak(&self) -> u64 {
        self.inner.peak.load(Ordering::Acquire)
    }

    /// Decode work charged so far.
    pub fn work(&self) -> u64 {
        self.inner.work.load(Ordering::Acquire)
    }
}

/// `docs/SECURITY_LIMITS.md`, rendered from [`LIMIT_TABLE`] and the two presets.
///
/// `rejected_by_browser` lists the corpus fixtures the browser preset refuses (an empty slice is
/// written as "none"); the caller measures it, since this crate has no corpus.
pub fn render_markdown(rejected_by_browser: &[String]) -> String {
    let mut s = String::new();
    s.push_str(
        "# Security limits\n\n\
This file is generated from `crates/pvec-container/src/limits.rs`. I do not edit it by hand: a test \
regenerates it and fails if the committed file differs by a single byte. To change it I change the \
constants and run `PVEC_BLESS_LIMITS=1 cargo test -p pvec-testkit --test issue_9_limits_corpus`.\n\n\
A `.pvec` file may be uploaded by anyone and decoded inside a browser tab, so every count and offset in \
it is attacker-controlled. `Limits` is the one configurable set of ceilings. Each is enforced before the \
allocation or the work it protects, and a file that passes one is refused with `LimitExceeded { limit, \
declared, ceiling }`: a distinct error, never a panic and never a silent clamp. The reader threads an \
`Accountant` through every buffer it makes, so the allocation budget is the sum of the live buffers and \
not one allocation at a time.\n\n\
The geometric ceilings cite NP-18 in `docs/NUMERIC_POLICY.md` and take their defaults from the constants \
of `pvec-numeric`. `Limits::validate` refuses a ceiling above NP-18's maximum, because the block parsers \
refuse a file past it whatever the configuration says. Lowering one is what the browser preset does; the \
other limits can be raised, and a test shows that raising one lets a fixture through that the default \
refused.\n\n\
## Defaults and the browser preset\n\n\
| rule | limit | field | default | browser |\n|---|---|---|---|---|\n",
    );
    for row in LIMIT_TABLE {
        let k = row.kind;
        let (default, browser) = match k {
            LimitKind::Extent => ("the artifact length".to_string(), "the artifact length".to_string()),
            _ => (Limits::DEFAULT.get(k).to_string(), Limits::BROWSER.get(k).to_string()),
        };
        let field = match k {
            LimitKind::Extent => "none".to_string(),
            _ => format!("`{}`", k.field()),
        };
        s.push_str(&format!("| {} | {:?} | {} | {} | {} |\n", k.rule(), k, field, default, browser));
    }
    s.push_str(
        "\n## Rationale and where each is enforced\n\n| rule | rationale | enforced |\n|---|---|---|\n",
    );
    for row in LIMIT_TABLE {
        s.push_str(&format!("| {} | {} | {} |\n", row.kind.rule(), row.rationale, row.enforced));
    }
    s.push_str(
        "\n## Component stream ceilings (the paint model)\n\n\
These are fixed by the layout of the stream, not fields of `Limits`: a decoder holds its input to them before it allocates from a count, and a writer refuses to write past them. Every one is a `COMP-03` unless noted.\n\
\n\
| stream | ceiling |\n\
|---|---|\n\
| 23 `paint_order` | 1,048,576 runs (12 bytes each, so 12 MiB), and at most 2^24 items once the runs are expanded; a run of zero items is `COMP-05`, an item named twice `COMP-21` |\n\
| 24 `stroke_paint` | 65,536 colours and 2^24 assignments (8 bytes each); no colours at all is `COMP-05`, keys that do not ascend `COMP-28` |\n\
| 25 `fill_paint` | 4,096 gradients, 1,024 tilings and 2^20 assignments; 2 to 256 stops a gradient (`COMP-05` below, `COMP-03` above); 1 to 256 primitives a tiling, 4,096 points a primitive, and 8,192 points in all (`COMP-27`) |\n\
| 26 `clip` | 65,536 clips and 2^20 runs; 1 to 4,096 entities in a boundary (`COMP-05` below, `COMP-03` above); chains 8 deep (`COMP-26`); a parent that is not an earlier clip is `COMP-25`; an item named by two runs is `COMP-21` |\n",
    );
    s.push_str(
        "\n## Refusal behaviour\n\n\
Every row is a `LimitExceeded` carrying the rule, the value the file declared and the ceiling. For \
`LIMIT-03` and `LIMIT-04` the declared value is the total the request would have brought the reader to. \
For `LIMIT-11` it is the first level over the ceiling, since the walk stops there; a composite that \
leads back to itself has no depth and is refused as a cycle (`ARN-21`) instead. The reader refuses \
before it reserves: a test per row uses a counting allocator to show that the size the file declared was \
never requested.\n\n\
A refusal leaves the reader usable. An accountant that refuses a reservation has reserved nothing, and \
the blocks already decoded keep their bytes until they are dropped.\n\n",
    );
    s.push_str("## What the browser preset refuses that the defaults admit\n\n");
    if rejected_by_browser.is_empty() {
        s.push_str(
            "Of the phase 2 corpus fixtures, none. The defaults admit every one, and so does the browser preset.\n",
        );
    } else {
        s.push_str("Of the phase 2 corpus fixtures, these (the defaults admit all of them):\n\n");
        for name in rejected_by_browser {
            s.push_str(&format!("* `{name}`\n"));
        }
    }
    s.push_str(
        "\nIn general the browser preset refuses an artifact over 4 GiB, a block over 64 MiB, a directory of more \
than 131072 blocks, an expansion ratio over 32, more than 2^24 entities, a curve of more than 16384 control \
points, composites nested more than 3 deep, and a reader that would hold more than 256 MiB of live decoded \
blocks.\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_cite_np_18_and_validate() {
        assert_eq!(Limits::DEFAULT.validate(), Ok(()));
        assert_eq!(Limits::BROWSER.validate(), Ok(()));
        assert_eq!(Limits::DEFAULT.max_control_points as usize, MAX_CONTROL_POINTS);
        assert_eq!(Limits::DEFAULT.max_nesting_depth as usize, MAX_NESTING_DEPTH);
    }

    #[test]
    fn a_ceiling_above_the_format_maximum_is_refused_not_clamped() {
        let l = Limits { max_control_points: (MAX_CONTROL_POINTS as u32) + 1, ..Limits::DEFAULT };
        let e = l.validate().unwrap_err();
        assert_eq!(e.limit, LimitKind::ControlPoints);
        assert_eq!(e.format_maximum, MAX_CONTROL_POINTS as u64);
    }

    #[test]
    fn the_accountant_counts_live_bytes_and_gives_them_back() {
        let a = Accountant::new(&Limits { max_alloc_bytes: 100, ..Limits::DEFAULT });
        let r1 = a.reserve(60).unwrap();
        let e = a.reserve(41).unwrap_err();
        assert_eq!((e.limit, e.declared, e.ceiling), (LimitKind::AllocationBudget, 101, 100));
        assert_eq!(a.live(), 60, "a refusal reserves nothing");
        drop(r1);
        assert_eq!(a.live(), 0);
        let _r2 = a.reserve(100).unwrap();
        assert_eq!(a.peak(), 100);
    }

    #[test]
    fn every_limit_has_a_unique_rule_a_field_and_a_row() {
        let mut rules: Vec<&str> = LimitKind::ALL.iter().map(|k| k.rule()).collect();
        rules.sort_unstable();
        rules.dedup();
        assert_eq!(rules.len(), LimitKind::ALL.len());
        for k in LimitKind::ALL {
            assert!(LIMIT_TABLE.iter().any(|r| r.kind == k), "{k:?} has no row");
        }
    }
}
