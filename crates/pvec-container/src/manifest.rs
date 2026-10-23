//! The artifact manifest.

use crate::header::check_range;
use crate::spec::{MAX_STRING_LEN, cap, dir, m};
use crate::{ContainerError, Header};
use pvec_capabilities::{CapabilityId, Declaration, Declared, Requirement, Version};
use pvec_numeric::check_value;

/// The unit of length of source space, with its ISO declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthUnit {
    /// Metre.
    Metre,
    /// Millimetre.
    Millimetre,
    /// Centimetre.
    Centimetre,
    /// Kilometre.
    Kilometre,
    /// Inch.
    Inch,
    /// Foot.
    Foot,
}

impl LengthUnit {
    /// Every unit, in code order.
    pub const ALL: [LengthUnit; 6] = [
        LengthUnit::Metre,
        LengthUnit::Millimetre,
        LengthUnit::Centimetre,
        LengthUnit::Kilometre,
        LengthUnit::Inch,
        LengthUnit::Foot,
    ];

    /// The code stored in the file.
    pub fn code(self) -> u16 {
        match self {
            LengthUnit::Metre => 1,
            LengthUnit::Millimetre => 2,
            LengthUnit::Centimetre => 3,
            LengthUnit::Kilometre => 4,
            LengthUnit::Inch => 5,
            LengthUnit::Foot => 6,
        }
    }

    /// The unit for a code.
    pub fn from_code(code: u16) -> Option<LengthUnit> {
        LengthUnit::ALL.into_iter().find(|u| u.code() == code)
    }

    /// Metres in one unit.
    pub fn metres_per_unit(self) -> f64 {
        match self {
            LengthUnit::Metre => 1.0,
            LengthUnit::Millimetre => 0.001,
            LengthUnit::Centimetre => 0.01,
            LengthUnit::Kilometre => 1000.0,
            LengthUnit::Inch => 0.0254,
            LengthUnit::Foot => 0.3048,
        }
    }

    /// The ISO 10303-41 length unit this one is declared as.
    pub fn iso_declaration(self) -> &'static str {
        match self {
            LengthUnit::Metre => "SI_UNIT($,.METRE.)",
            LengthUnit::Millimetre => "SI_UNIT(.MILLI.,.METRE.)",
            LengthUnit::Centimetre => "SI_UNIT(.CENTI.,.METRE.)",
            LengthUnit::Kilometre => "SI_UNIT(.KILO.,.METRE.)",
            LengthUnit::Inch => "CONVERSION_BASED_UNIT('INCH',0.0254)",
            LengthUnit::Foot => "CONVERSION_BASED_UNIT('FOOT',0.3048)",
        }
    }
}

/// Axis-aligned source bounds, in source space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// `[min_x, min_y]`.
    pub min: [f64; 2],
    /// `[max_x, max_y]`.
    pub max: [f64; 2],
}

/// An offset and length in the artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    /// Offset.
    pub offset: u64,
    /// Length.
    pub length: u64,
}

/// A pointer to a directory, by kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryPointer {
    /// What the directory is (`KIND_BLOCK`, `KIND_COMPONENT`, `KIND_SPATIAL`, or a later kind).
    pub kind: u32,
    /// Offset.
    pub offset: u64,
    /// Length.
    pub length: u64,
}

impl DirectoryPointer {
    /// Block directory (P4.2).
    pub const KIND_BLOCK: u32 = 1;
    /// Component directory (P4.3).
    pub const KIND_COMPONENT: u32 = 2;
    /// Spatial directory (P5.1).
    pub const KIND_SPATIAL: u32 = 3;
}

/// Caller-supplied artifact metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    /// Producer id.
    pub producer_id: String,
    /// Producer version.
    pub producer_version: String,
    /// Fixture or corpus revision, where there is one.
    pub corpus_revision: Option<String>,
}

/// The parsed manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Unit of length.
    pub unit: LengthUnit,
    /// Source bounds.
    pub bounds: Bounds,
    /// The lowest reader version the file may be opened with.
    pub min_reader: Version,
    /// Declared capabilities.
    pub capabilities: Vec<Declared>,
    /// Directory pointers.
    pub directories: Vec<DirectoryPointer>,
    /// Where P4.6's per-block checksum table lives, if written.
    pub block_checksums: Option<ByteRange>,
    /// P4.6's artifact SHA-256, if written.
    pub artifact_digest: Option<[u8; 32]>,
    /// Producer metadata.
    pub metadata: Metadata,
}

impl Manifest {
    /// The fixed part, in bytes.
    pub const FIXED_LEN: usize = 128;
    /// The largest manifest a reader will fetch.
    pub const MAX_LEN: u64 = 1 << 20;

    /// Parses and validates the manifest bytes the header points at.
    ///
    /// `bytes` is the manifest as fetched (at least `header.manifest_length` bytes). Every count is
    /// compared with the bytes that are really there before it sizes anything.
    pub fn parse(bytes: &[u8], header: &Header, artifact_len: u64) -> Result<Manifest, ContainerError> {
        let got = bytes.len() as u64;
        let truncated = |needed: u64| ContainerError::Truncated { needed, got };
        let declared = header.manifest_length.min(Manifest::MAX_LEN);
        if got < declared {
            return Err(truncated(declared));
        }
        let b = &bytes[..declared as usize];
        if b.len() < Manifest::FIXED_LEN {
            return Err(truncated(Manifest::FIXED_LEN as u64));
        }

        let flags = u16_at(b, m::FLAGS);
        if flags & !m::FLAG_DIGEST != 0 {
            return Err(ContainerError::InvalidFlags { bits: flags });
        }
        for (at, len) in [(m::RESERVED_A, 2), (m::RESERVED_B, 4), (m::RESERVED_C, 4)] {
            if b[at..at + len].iter().any(|x| *x != 0) {
                return Err(ContainerError::NonZeroReserved { field: "manifest reserved" });
            }
        }

        let code = u16_at(b, m::UNIT);
        let unit = LengthUnit::from_code(code).ok_or(ContainerError::UnknownLengthUnit { code })?;
        if f64_at(b, m::METRES_PER_UNIT).to_bits() != unit.metres_per_unit().to_bits() {
            return Err(ContainerError::UnitFactorMismatch);
        }

        let mut bound = [0.0f64; 4];
        for (i, (at, field)) in
            [(m::MIN_X, "min_x"), (m::MIN_Y, "min_y"), (m::MAX_X, "max_x"), (m::MAX_Y, "max_y")]
                .into_iter()
                .enumerate()
        {
            bound[i] = check_value(f64_at(b, at)).map_err(|rule| ContainerError::Bound { field, rule })?;
        }
        let bounds = Bounds { min: [bound[0], bound[1]], max: [bound[2], bound[3]] };
        if bounds.min[0] > bounds.max[0] {
            return Err(ContainerError::InvertedBounds { axis: "x" });
        }
        if bounds.min[1] > bounds.max[1] {
            return Err(ContainerError::InvertedBounds { axis: "y" });
        }

        let min_reader = Version::new(
            u16_at(b, m::MIN_READER_MAJOR),
            u16_at(b, m::MIN_READER_MINOR),
            u16_at(b, m::MIN_READER_PATCH),
        );

        let mut lens = [0u32; 3];
        for (i, (at, field)) in [
            (m::PRODUCER_ID_LEN, "producer_id"),
            (m::PRODUCER_VERSION_LEN, "producer_version"),
            (m::CORPUS_REVISION_LEN, "corpus_revision"),
        ]
        .into_iter()
        .enumerate()
        {
            lens[i] = u32_at(b, at);
            if lens[i] > MAX_STRING_LEN {
                return Err(ContainerError::StringTooLong { field, length: lens[i] });
            }
        }
        if lens[0] == 0 {
            return Err(ContainerError::MissingProducer);
        }

        // Sizes are summed in u64 from u32 counts, so a lying count cannot overflow, and nothing is
        // allocated until the whole implied layout is known to fit.
        let cap_count = u32_at(b, m::CAPABILITY_COUNT) as u64;
        let dir_count = u32_at(b, m::DIRECTORY_COUNT) as u64;
        let cap_start = Manifest::FIXED_LEN as u64;
        let dir_start = cap_start + cap_count * cap::SIZE as u64;
        let str_start = dir_start + dir_count * dir::SIZE as u64;
        let str_len: u64 = lens.iter().map(|l| *l as u64).sum();
        let end = (str_start + str_len + 7) & !7;
        if end > b.len() as u64 {
            return Err(truncated(end));
        }
        if header.directory_table_offset != header.manifest_offset + dir_start
            || header.directory_table_length != dir_count * dir::SIZE as u64
        {
            return Err(ContainerError::DirectoryTableMismatch);
        }

        let floor = header.header_len as u64;
        let mut capabilities: Vec<Declared> = Vec::with_capacity(cap_count as usize);
        for i in 0..cap_count as usize {
            let e = &b[cap_start as usize + i * cap::SIZE..][..cap::SIZE];
            let id = u32_at(e, cap::ID);
            if id == 0 {
                return Err(ContainerError::ZeroCapabilityId);
            }
            if capabilities.iter().any(|c| c.id.get() == id) {
                return Err(ContainerError::DuplicateCapability { id });
            }
            let requirement = match e[cap::REQUIREMENT] {
                cap::REQUIRED => Requirement::Required,
                cap::OPTIONAL => Requirement::Optional,
                value => return Err(ContainerError::InvalidRequirement { id, value }),
            };
            if e[cap::RESERVED_A] != 0 || u32_at(e, cap::RESERVED_B) != 0 {
                return Err(ContainerError::NonZeroReserved { field: "capability reserved" });
            }
            capabilities.push(Declared {
                id: CapabilityId::new(id),
                requirement,
                version: Version::new(u16_at(e, cap::MAJOR), u16_at(e, cap::MINOR), u16_at(e, cap::PATCH)),
            });
        }

        let mut directories: Vec<DirectoryPointer> = Vec::with_capacity(dir_count as usize);
        for i in 0..dir_count as usize {
            let e = &b[dir_start as usize + i * dir::SIZE..][..dir::SIZE];
            if u32_at(e, dir::RESERVED) != 0 {
                return Err(ContainerError::NonZeroReserved { field: "directory reserved" });
            }
            let (offset, length) = (u64_at(e, dir::OFFSET), u64_at(e, dir::LENGTH));
            check_range("directory", offset, length, floor, artifact_len)?;
            directories.push(DirectoryPointer { kind: u32_at(e, dir::KIND), offset, length });
        }

        let (cs_offset, cs_length) =
            (u64_at(b, m::BLOCK_CHECKSUMS_OFFSET), u64_at(b, m::BLOCK_CHECKSUMS_LENGTH));
        let block_checksums = if cs_offset == 0 && cs_length == 0 {
            None
        } else {
            check_range("block_checksums", cs_offset, cs_length, floor, artifact_len)?;
            Some(ByteRange { offset: cs_offset, length: cs_length })
        };

        let mut digest = [0u8; 32];
        digest.copy_from_slice(&b[m::ARTIFACT_DIGEST..m::ARTIFACT_DIGEST + 32]);
        let artifact_digest = if flags & m::FLAG_DIGEST != 0 {
            Some(digest)
        } else if digest.iter().any(|x| *x != 0) {
            return Err(ContainerError::NonZeroReserved { field: "artifact_digest" });
        } else {
            None
        };

        let mut at = str_start as usize;
        let mut text = |len: u32, field: &'static str| -> Result<String, ContainerError> {
            let s = core::str::from_utf8(&b[at..at + len as usize])
                .map_err(|_| ContainerError::InvalidUtf8 { field })?
                .to_string();
            at += len as usize;
            Ok(s)
        };
        let producer_id = text(lens[0], "producer_id")?;
        let producer_version = text(lens[1], "producer_version")?;
        let corpus = text(lens[2], "corpus_revision")?;
        if b[at..end as usize].iter().any(|x| *x != 0) {
            return Err(ContainerError::NonZeroReserved { field: "manifest padding" });
        }
        if header.minor == Header::MINOR && (b.len() as u64) > end {
            return Err(ContainerError::TrailingBytes { extra: b.len() as u64 - end });
        }

        Ok(Manifest {
            unit,
            bounds,
            min_reader,
            capabilities,
            directories,
            block_checksums,
            artifact_digest,
            metadata: Metadata {
                producer_id,
                producer_version,
                corpus_revision: if lens[2] == 0 { None } else { Some(corpus) },
            },
        })
    }

    /// The canonical encoding.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; Manifest::FIXED_LEN];
        let put16 = |b: &mut Vec<u8>, at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
        let put32 = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let put64 = |b: &mut Vec<u8>, at: usize, v: u64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());
        let putf = |b: &mut Vec<u8>, at: usize, v: f64| {
            let v = if v == 0.0 { 0.0 } else { v };
            b[at..at + 8].copy_from_slice(&v.to_le_bytes())
        };
        put16(&mut b, m::UNIT, self.unit.code());
        put16(&mut b, m::FLAGS, if self.artifact_digest.is_some() { m::FLAG_DIGEST } else { 0 });
        put32(&mut b, m::CAPABILITY_COUNT, self.capabilities.len() as u32);
        putf(&mut b, m::METRES_PER_UNIT, self.unit.metres_per_unit());
        putf(&mut b, m::MIN_X, self.bounds.min[0]);
        putf(&mut b, m::MIN_Y, self.bounds.min[1]);
        putf(&mut b, m::MAX_X, self.bounds.max[0]);
        putf(&mut b, m::MAX_Y, self.bounds.max[1]);
        put16(&mut b, m::MIN_READER_MAJOR, self.min_reader.major);
        put16(&mut b, m::MIN_READER_MINOR, self.min_reader.minor);
        put16(&mut b, m::MIN_READER_PATCH, self.min_reader.patch);
        put32(&mut b, m::DIRECTORY_COUNT, self.directories.len() as u32);
        if let Some(r) = self.block_checksums {
            put64(&mut b, m::BLOCK_CHECKSUMS_OFFSET, r.offset);
            put64(&mut b, m::BLOCK_CHECKSUMS_LENGTH, r.length);
        }
        if let Some(d) = self.artifact_digest {
            b[m::ARTIFACT_DIGEST..m::ARTIFACT_DIGEST + 32].copy_from_slice(&d);
        }
        let corpus = self.metadata.corpus_revision.as_deref().unwrap_or("");
        put32(&mut b, m::PRODUCER_ID_LEN, self.metadata.producer_id.len() as u32);
        put32(&mut b, m::PRODUCER_VERSION_LEN, self.metadata.producer_version.len() as u32);
        put32(&mut b, m::CORPUS_REVISION_LEN, corpus.len() as u32);
        for c in &self.capabilities {
            let mut e = [0u8; cap::SIZE];
            e[cap::ID..cap::ID + 4].copy_from_slice(&c.id.get().to_le_bytes());
            e[cap::MAJOR..cap::MAJOR + 2].copy_from_slice(&c.version.major.to_le_bytes());
            e[cap::MINOR..cap::MINOR + 2].copy_from_slice(&c.version.minor.to_le_bytes());
            e[cap::PATCH..cap::PATCH + 2].copy_from_slice(&c.version.patch.to_le_bytes());
            e[cap::REQUIREMENT] = match c.requirement {
                Requirement::Required => cap::REQUIRED,
                Requirement::Optional => cap::OPTIONAL,
            };
            b.extend_from_slice(&e);
        }
        for d in &self.directories {
            let mut e = [0u8; dir::SIZE];
            e[dir::KIND..dir::KIND + 4].copy_from_slice(&d.kind.to_le_bytes());
            e[dir::OFFSET..dir::OFFSET + 8].copy_from_slice(&d.offset.to_le_bytes());
            e[dir::LENGTH..dir::LENGTH + 8].copy_from_slice(&d.length.to_le_bytes());
            b.extend_from_slice(&e);
        }
        b.extend_from_slice(self.metadata.producer_id.as_bytes());
        b.extend_from_slice(self.metadata.producer_version.as_bytes());
        b.extend_from_slice(corpus.as_bytes());
        while b.len() % 8 != 0 {
            b.push(0);
        }
        b
    }

    /// The capability declaration P4.7 negotiates against.
    pub fn declaration(&self) -> Declaration {
        Declaration { min_reader: self.min_reader, capabilities: self.capabilities.clone() }
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    f64::from_bits(u64_at(b, at))
}
