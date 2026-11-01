//! Issue #251: a test-support tiler writes a large raster `.pvec` from a seed, byte for byte the same on any
//! machine, with seeded linework over it.

use pvec_container::blocks::sniff::max_tile_bytes;
use pvec_container::limits::Limits;
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_testkit::large_raster::large_raster;
use pvec_testkit::tiler::Shape;
use std::path::PathBuf;
use std::sync::OnceLock;

const SMALL: Shape = Shape { width: 1500, height: 900, tile_edge: 256, channels: 1, bit_depth: 8 };
const BIG: Shape = Shape { width: 8192, height: 8192, tile_edge: 1024, channels: 1, bit_depth: 8 };

fn big() -> &'static Vec<u8> {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| large_raster(7, BIG, 200, 20))
}

fn open(bytes: &[u8]) -> ContainerReader<MemorySource<Vec<u8>>> {
    ContainerReader::open(MemorySource::new(bytes.to_vec()), ReaderConfig::default()).unwrap()
}

#[test]
fn two_runs_with_the_same_arguments_are_equal() {
    assert_eq!(large_raster(1, SMALL, 40, 6), large_raster(1, SMALL, 40, 6));
}

#[test]
fn different_seeds_differ() {
    assert_ne!(large_raster(1, SMALL, 40, 6), large_raster(2, SMALL, 40, 6));
}

#[test]
fn the_linework_counts_change_the_file() {
    assert_ne!(large_raster(1, SMALL, 40, 6), large_raster(1, SMALL, 41, 6));
    assert_ne!(large_raster(1, SMALL, 40, 6), large_raster(1, SMALL, 40, 7));
}

#[test]
fn the_reader_opens_it_and_the_index_lists_every_tile_of_every_level() {
    let reader = open(&large_raster(3, SMALL, 40, 6));
    reader.validate().unwrap();
    reader.validate_raster(0).unwrap();
    let index = reader.raster_index(0).unwrap();
    index.check_grid(SMALL.width, SMALL.height, SMALL.tile_edge).unwrap();
    let grid = SMALL.grid();
    assert_eq!(index.levels().len() as u32, SMALL.levels());
    let total: u32 = grid.iter().map(|(c, r)| c * r).sum();
    assert_eq!(index.tiles().len() as u32, total);
    assert_eq!(index.stored(), total, "every tile is stored");
    for (l, (cols, rows)) in grid.into_iter().enumerate() {
        for row in 0..rows {
            for col in 0..cols {
                let t = reader.read_raster_tile(0, l as u32, col, row).unwrap();
                assert!(t.is_some(), "tile {l}/{col}/{row}");
            }
        }
    }
}

#[test]
fn the_file_has_linework_over_the_raster() {
    let reader = open(&large_raster(3, SMALL, 40, 6));
    assert_eq!(reader.validate().unwrap().entities, 46, "40 lines and 6 curves");
}

#[test]
fn the_tile_and_level_counts_match_shape_at_8192() {
    let reader = open(big());
    let index = reader.raster_index(0).unwrap();
    assert_eq!(BIG.levels(), 4);
    assert_eq!(index.levels().len() as u32, BIG.levels());
    assert_eq!(index.tiles().len(), 64 + 16 + 4 + 1);
    index.check_grid(BIG.width, BIG.height, BIG.tile_edge).unwrap();
}

#[test]
fn the_output_stays_within_the_container_limits_at_8192() {
    let bytes = big();
    let limits = Limits::DEFAULT;
    assert!((bytes.len() as u64) < limits.max_artifact_bytes);
    assert!(bytes.len() < 100 << 20, "about 90 MB, not more: {}", bytes.len());
    let reader = open(bytes);
    reader.validate_raster(0).unwrap();
    let index = reader.raster_index(0).unwrap();
    assert!(index.tiles().len() as u64 <= u64::from(limits.max_raster_tiles));
    let ceiling = max_tile_bytes(BIG.tile_edge, limits.max_tile_bytes);
    for t in index.tiles() {
        assert!(u64::from(t.length) <= ceiling);
    }
}

fn golden() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/large-raster-digests.txt")
}

#[test]
fn the_bytes_are_stable_against_the_committed_digests() {
    let line = |name: &str, bytes: &[u8]| format!("{name} {}\n", pvec_corpus::sha256::hex_digest(bytes));
    let got = line("seed1-1500x900-e256-l40-c6", &large_raster(1, SMALL, 40, 6))
        + &line("seed7-8192x8192-e1024-l200-c20", big());
    if std::env::var_os("PVEC_BLESS_LARGE_RASTER").is_some() {
        std::fs::write(golden(), &got).unwrap();
    }
    let committed = std::fs::read_to_string(golden()).unwrap_or_default();
    assert_eq!(committed, got, "tests/golden/large-raster-digests.txt (PVEC_BLESS_LARGE_RASTER=1 writes it)");
}

#[test]
fn a_one_bit_raster_is_smaller_and_still_valid() {
    let one = Shape { bit_depth: 1, ..SMALL };
    let (a, b) = (large_raster(1, SMALL, 10, 2), large_raster(1, one, 10, 2));
    assert!(b.len() < a.len());
    open(&b).validate_raster(0).unwrap();
}
