//! Codec sweep over the geometry blocks of one `.pvec` file (P8.3).
//!
//!   measure ENTRY FILE PLATFORM
//!
//! The file is written with no compression, so each geometry block (hot, advanced records, arenas) is its raw
//! bytes. Per block means each block is compressed and decompressed alone, as the container does; whole file
//! means all the geometry blocks as one buffer. Rows are tidy
//! (`table,entry,platform,arm,param,metric,value,note`) for `docs/bench/layout.csv`. Timings are the best of three
//! runs (one for the slowest setting on the biggest input), in MiB of raw bytes per second.
#![allow(missing_docs)]

use pvec_container::directory::BlockKind;
use pvec_container::source::MemorySource;
use pvec_container::{ContainerReader, ReaderConfig};
use std::time::Instant;

type Compress = Box<dyn Fn(&[u8]) -> Vec<u8>>;
type Decompress = Box<dyn Fn(&[u8], usize) -> Vec<u8>>;

fn best<T>(runs: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut out = None;
    let mut fastest = f64::INFINITY;
    for _ in 0..runs {
        let t = Instant::now();
        let v = f();
        fastest = fastest.min(t.elapsed().as_secs_f64());
        out = Some(v);
    }
    (fastest, out.expect("at least one run"))
}

fn zstd_at(level: i32) -> (Compress, Decompress) {
    (
        Box::new(move |b| zstd::bulk::compress(b, level).expect("zstd")),
        Box::new(|b, n| zstd::bulk::decompress(b, n).expect("zstd")),
    )
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (entry, file, platform) = (&a[1], &a[2], &a[3]);
    let bytes = std::fs::read(file).expect("the file");
    let r =
        ContainerReader::open(MemorySource::new(bytes.as_slice()), ReaderConfig::default()).expect("opens");
    let blocks: Vec<&[u8]> = r
        .entries()
        .iter()
        .filter(|e| matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)))
        .map(|e| {
            assert_eq!(e.codec, 0, "the file is written uncompressed");
            &bytes[e.offset as usize..e.offset as usize + e.stored_length as usize]
        })
        .collect();
    let whole: Vec<u8> = blocks.concat();
    let raw = whole.len() as f64;
    let mib = raw / (1024.0 * 1024.0);
    let note = format!("{} geometry blocks and {} raw bytes", blocks.len(), whole.len());
    let row = |arm: &str, scope: &str, metric: &str, v: f64| {
        println!("compression,{entry},{platform},{arm},{scope},{metric},{v},{note}");
    };
    let big = whole.len() > 100 << 20;

    let lz4: (Compress, Decompress) = (
        Box::new(lz4_flex::block::compress),
        Box::new(|b, n| {
            let mut out = vec![0u8; n];
            lz4_flex::block::decompress_into(b, &mut out).expect("lz4");
            out
        }),
    );
    let none: (Compress, Decompress) = (Box::new(|b| b.to_vec()), Box::new(|b, _| b.to_vec()));
    let codecs =
        [("none", none), ("lz4", lz4), ("zstd3", zstd_at(3)), ("zstd9", zstd_at(9)), ("zstd19", zstd_at(19))];
    for (name, (compress, decompress)) in &codecs {
        let runs = if *name == "zstd19" && big { 1 } else { 3 };
        let (ct, packed) = best(runs, || blocks.iter().map(|b| compress(b)).collect::<Vec<_>>());
        let stored: usize = packed.iter().map(Vec::len).sum();
        let (dt, back) = best(runs, || {
            packed.iter().zip(&blocks).map(|(p, b)| decompress(p, b.len())).collect::<Vec<_>>()
        });
        assert!(back.iter().zip(&blocks).all(|(x, y)| x.as_slice() == *y), "{name}: per block round trip");
        row(name, "block", "ratio", stored as f64 / raw);
        row(name, "block", "stored_bytes", stored as f64);
        row(name, "block", "compress_mib_s", mib / ct);
        row(name, "block", "decompress_mib_s", mib / dt);

        let (ct, packed) = best(runs, || compress(&whole));
        let (dt, back) = best(runs, || decompress(&packed, whole.len()));
        assert!(back == whole, "{name}: whole file round trip");
        row(name, "file", "ratio", packed.len() as f64 / raw);
        row(name, "file", "stored_bytes", packed.len() as f64);
        row(name, "file", "compress_mib_s", mib / ct);
        row(name, "file", "decompress_mib_s", mib / dt);
    }
}
