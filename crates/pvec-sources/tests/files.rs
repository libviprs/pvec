//! A file and a positional read give the same artifact as memory does (issue P4.5), and a file that
//! changes under the reader is an error and not a short decode.

use pvec_container::reader::{BlockId, ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::{BlockSource, MemorySource, ReadAtSource, SourceError};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use pvec_sources::{FileSource, PositionalFile};
use std::path::PathBuf;

fn artifact() -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "files-test".into(),
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

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("pvec-sources-{}-{name}.pvec", std::process::id()))
}

#[test]
fn a_file_a_positional_read_and_memory_give_the_same_artifact() {
    let bytes = artifact();
    let path = temp("same");
    std::fs::write(&path, &bytes).unwrap();
    let want = ContainerReader::open(MemorySource::new(bytes.clone()), ReaderConfig::default())
        .unwrap()
        .validate()
        .unwrap();
    assert!(want.entities >= 40);

    let file = ContainerReader::open(FileSource::open(&path).unwrap(), ReaderConfig::default()).unwrap();
    assert_eq!(file.validate().unwrap(), want);

    let (positional, len) = PositionalFile::open(&path).unwrap();
    let at = ContainerReader::open(ReadAtSource::new(positional, len), ReaderConfig::default()).unwrap();
    assert_eq!(at.validate().unwrap(), want);
    assert_eq!(
        at.read_block(BlockId(1)).unwrap().as_bytes(),
        file.read_block(BlockId(1)).unwrap().as_bytes()
    );
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_range_past_the_end_of_a_file_and_a_file_cut_short_after_opening_are_errors() {
    let bytes = artifact();
    let path = temp("cut");
    std::fs::write(&path, &bytes).unwrap();
    let source = FileSource::open(&path).unwrap();
    assert_eq!(source.len(), bytes.len() as u64);
    assert!(matches!(source.read_range(bytes.len() as u64 - 4, 8), Err(SourceError::OutOfRange { .. })));
    assert!(matches!(source.read_range(u64::MAX, 8), Err(SourceError::OutOfRange { .. })));
    let reader = ContainerReader::open(source, ReaderConfig::default()).unwrap();
    // the file is cut to the header and the manifest after the reader opened it
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(200).unwrap();
    let held_by_the_layout = reader.accountant().live();
    let err = reader.read_block(BlockId(0)).unwrap_err();
    assert!(matches!(err, ReadError::Source(SourceError::Short { .. })), "{err:?}");
    assert_eq!(reader.accountant().live(), held_by_the_layout, "a short read holds no memory");
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_missing_file_is_an_error() {
    assert!(matches!(FileSource::open(temp("missing")), Err(SourceError::Io(_))));
    assert!(matches!(PositionalFile::open(temp("missing")), Err(SourceError::Io(_))));
}
