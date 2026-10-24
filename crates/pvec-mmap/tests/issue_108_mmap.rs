//! Issue #108: a memory-mapped source for the reader, held to the same checks as every other source.

use pvec_container::reader::{BlockId, ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::{BlockSource, MemorySource, ReadAtSource, SourceError};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use pvec_mmap::MmapSource;
use pvec_sources::{FileSource, PositionalFile};
use pvec_testkit::hostile::{
    Limits, NoMeter, Reader, Refusal, Requires, Verdict, bases, catalog, run,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn artifact() -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "mmap-test".into(),
            producer_version: "0".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    });
    for i in 0..40u32 {
        let meta = EntityMeta {
            id: EntityId::new(i).unwrap(),
            style: 0,
            cell: i / 10,
            bounds: Bounds { min: [0.0, 0.0], max: [50.0, 50.0] },
        };
        let f = f64::from(i);
        w.push_line(meta, [f, 0.0], [f + 1.0, 2.0 + f]).unwrap();
    }
    w.finish().unwrap()
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("pvec-mmap-{}-{n}-{name}.pvec", std::process::id()))
}

#[test]
fn a_map_a_file_a_positional_read_and_memory_give_the_same_artifact() {
    let bytes = artifact();
    let path = temp("same");
    std::fs::write(&path, &bytes).unwrap();
    let want = ContainerReader::open(MemorySource::new(bytes.clone()), ReaderConfig::default())
        .unwrap()
        .validate()
        .unwrap();
    assert!(want.entities >= 40);

    let mapped = MmapSource::open(&path).unwrap();
    assert_eq!(mapped.len(), bytes.len() as u64);
    let m = ContainerReader::open(mapped, ReaderConfig::default()).unwrap();
    assert_eq!(m.validate().unwrap(), want);

    let f = ContainerReader::open(FileSource::open(&path).unwrap(), ReaderConfig::default()).unwrap();
    let (positional, len) = PositionalFile::open(&path).unwrap();
    let p = ContainerReader::open(ReadAtSource::new(positional, len), ReaderConfig::default()).unwrap();
    for id in [0, 1, 2] {
        let a = m.read_block(BlockId(id)).unwrap();
        assert_eq!(a.as_bytes(), f.read_block(BlockId(id)).unwrap().as_bytes());
        assert_eq!(a.as_bytes(), p.read_block(BlockId(id)).unwrap().as_bytes());
    }
    m.verify_artifact_digest().unwrap();
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn ranges_are_checked_before_the_map_is_touched() {
    let bytes = artifact();
    let path = temp("range");
    std::fs::write(&path, &bytes).unwrap();
    let source = MmapSource::open(&path).unwrap();
    let end = bytes.len() as u64;
    assert_eq!(&*source.read_range(0, 8).unwrap(), &bytes[..8]);
    assert_eq!(&*source.read_range(end - 4, 4).unwrap(), &bytes[bytes.len() - 4..]);
    assert!(source.read_range(end, 0).unwrap().is_empty());
    for (offset, len) in [(end - 4, 8), (end, 1), (end + 1, 0), (u64::MAX, 8), (u64::MAX - 3, u32::MAX)] {
        assert!(
            matches!(source.read_range(offset, len), Err(SourceError::OutOfRange { .. })),
            "{offset}+{len}"
        );
    }
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_missing_file_and_a_directory_are_errors_and_an_empty_file_is_an_empty_source() {
    assert!(matches!(MmapSource::open(temp("missing")), Err(SourceError::Io(_))));
    assert!(matches!(MmapSource::open(std::env::temp_dir()), Err(SourceError::Io(_))));
    let path = temp("empty");
    std::fs::write(&path, b"").unwrap();
    let source = MmapSource::open(&path).unwrap();
    assert_eq!(source.len(), 0);
    assert!(source.is_empty());
    assert!(matches!(source.read_range(0, 1), Err(SourceError::OutOfRange { .. })));
    assert!(ContainerReader::open(source, ReaderConfig::default()).is_err(), "an empty file is not an artifact");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_file_cut_short_after_the_map_was_made_is_an_error_and_not_a_fault() {
    let bytes = artifact();
    let path = temp("cut");
    std::fs::write(&path, &bytes).unwrap();
    let source = MmapSource::open(&path).unwrap();
    let reader = ContainerReader::open(source, ReaderConfig::default()).unwrap();
    // the file is cut to the header and the manifest after the reader opened it; reading a block out of
    // the map now would touch pages that no longer exist
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(200).unwrap();
    let held_by_the_layout = reader.accountant().live();
    let err = reader.read_block(BlockId(0)).unwrap_err();
    assert!(matches!(err, ReadError::Source(SourceError::Short { .. })), "{err:?}");
    assert_eq!(reader.accountant().live(), held_by_the_layout, "a short read holds no memory");
    // and the part that is still there is still readable
    assert!(reader.validate().is_err());
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_file_that_grows_after_the_map_was_made_is_read_as_it_was() {
    let bytes = artifact();
    let path = temp("grow");
    std::fs::write(&path, &bytes).unwrap();
    let source = MmapSource::open(&path).unwrap();
    let mut longer = bytes.clone();
    longer.extend_from_slice(&[0xAA; 4096]);
    std::fs::write(&path, &longer).unwrap();
    assert_eq!(source.len(), bytes.len() as u64, "the length is the one at open");
    assert!(matches!(source.read_range(bytes.len() as u64, 1), Err(SourceError::OutOfRange { .. })));
    assert_eq!(&*source.read_range(0, 16).unwrap(), &bytes[..16]);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn the_source_can_be_shared_between_threads() {
    fn is_send_sync<T: Send + Sync>() {}
    is_send_sync::<MmapSource>();
    let bytes = artifact();
    let path = temp("threads");
    std::fs::write(&path, &bytes).unwrap();
    let source = Arc::new(MmapSource::open(&path).unwrap());
    let handles: Vec<_> = (0..4)
        .map(|t| {
            let (s, b) = (source.clone(), bytes.clone());
            std::thread::spawn(move || {
                for i in 0..200usize {
                    let at = (i * 7 + t) % (b.len() - 16);
                    assert_eq!(&*s.read_range(at as u64, 16).unwrap(), &b[at..at + 16]);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    std::fs::remove_file(&path).unwrap();
}

/// The safe reader, over a file the case is written to and mapped from.
struct Mapped;

impl Reader for Mapped {
    fn name(&self) -> String {
        "mmap".to_string()
    }

    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse, Requires::Limits, Requires::Integrity, Requires::Negotiation]
    }

    fn read(&self, bytes: &[u8]) -> Verdict {
        let path = temp("case");
        if let Err(e) = std::fs::write(&path, bytes) {
            return Verdict::Refused(Refusal { rule: "IO".to_string(), detail: e.to_string() });
        }
        let read = || -> Result<(), ReadError> {
            let source = MmapSource::open(&path).map_err(ReadError::Source)?;
            let reader = ContainerReader::open(source, ReaderConfig::default())?;
            reader.validate()?;
            reader.verify_artifact_digest()?;
            Ok(())
        };
        let verdict = match read() {
            Ok(()) => Verdict::Accepted,
            Err(e) => Verdict::Refused(Refusal { rule: e.rule(), detail: e.to_string() }),
        };
        let _ = std::fs::remove_file(&path);
        verdict
    }
}

#[test]
fn the_hostile_corpus_and_its_sweeps_give_the_same_verdicts_through_a_map_as_through_memory() {
    let bundle = pvec_corpus::Bundle::vendored().expect("the vendored bundle opens");
    let b = bases(&bundle);
    let c = catalog(&b);
    let total = c.len();
    let report = run(Arc::new(Mapped), Arc::new(NoMeter), Arc::new(b), Arc::new(c), Limits::default());
    assert!(report.passed(), "{}", report.summary());
    assert!(report.ran > 10_000, "ran only {}", report.ran);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    assert_eq!(report.ran, total);
}
