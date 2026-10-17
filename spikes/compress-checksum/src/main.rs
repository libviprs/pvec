//! Per-block compression and checksum measurements on layout B's encoded blocks.
//!
//!   measure compress <fixture>        compressed bytes and throughput, with and without the shuffle
//!   measure checksum <fixture>        checksum throughput over the blocks
//!   measure dump <fixture> <dir>      writes raw / deflate-1 / lz4 block buffers for the wasm harness

use compress_checksum_spike::*;
use layouts_spike::common::{block_bytes, blocks, build_file, parse_dir};
use layouts_spike::packed::Packed;
use pvec_fixtures::generate;

fn cpu() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid, writable `timespec`; CLOCK_PROCESS_CPUTIME_ID exists on Linux.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1_000_000_000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn raw_blocks(fixture: &str) -> (Vec<Vec<u8>>, usize) {
    let d = match fixture {
        "small-synthetic" => generate::small_synthetic(),
        "spline-heavy" => generate::spline_heavy(),
        "million" => generate::million(),
        "straddler" => generate::straddler(),
        other => panic!("unknown fixture {other}"),
    };
    let file = build_file::<Packed>(&blocks(&d));
    let dir = parse_dir(&file).expect("directory");
    let v: Vec<Vec<u8>> = dir.iter().map(|e| block_bytes(&file, e).to_vec()).collect();
    (v, file.len())
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (mode, fixture) = (a[1].as_str(), a[2].as_str());
    let (raw, file_len) = raw_blocks(fixture);
    let total: usize = raw.iter().map(Vec::len).sum();
    let mb = total as f64 / 1_000_000.0;
    println!("load average: {}", std::fs::read_to_string("/proc/loadavg").unwrap_or_default().trim());
    println!(
        "## {fixture}: {} blocks, {} block bytes (file {file_len}), mean block {} bytes\n",
        raw.len(),
        total,
        total / raw.len()
    );
    match mode {
        "compress" => {
            // Whole-stream deflate for reference: what independence between blocks costs.
            let mut whole = Vec::with_capacity(total);
            for b in &raw {
                whole.extend_from_slice(b);
            }
            let w1 = Codec::Deflate1.compress(&whole).len();
            let w6 = Codec::Deflate6.compress(&whole).len();
            println!(
                "whole-stream reference (one deflate stream over all blocks, no random access): level 1 {w1} bytes ({:.3}), level 6 {w6} bytes ({:.3})\n",
                w1 as f64 / total as f64,
                w6 as f64 / total as f64
            );
            println!(
                "| codec | pre-filter | compressed bytes | ratio to raw | compress MB/s | decompress MB/s |"
            );
            println!("|---|---|---|---|---|---|");
            let codecs = [Codec::Deflate1, Codec::Deflate6, Codec::Lz4, Codec::Zstd1, Codec::Zstd3];
            for shuffled in [false, true] {
                let input: Vec<Vec<u8>> =
                    if shuffled { raw.iter().map(|b| shuffle8(b)).collect() } else { raw.clone() };
                for c in codecs {
                    let mut t = vec![];
                    let mut comp: Vec<Vec<u8>> = vec![];
                    for _ in 0..3 {
                        let s = cpu();
                        comp = input.iter().map(|b| c.compress(b)).collect();
                        t.push(cpu() - s);
                    }
                    let cbytes: usize = comp.iter().map(Vec::len).sum();
                    let mut td = vec![];
                    for _ in 0..3 {
                        let s = cpu();
                        let mut sink = 0usize;
                        for (cb, rb) in comp.iter().zip(&input) {
                            let out = c.decompress(cb, rb.len()).expect("round trip");
                            sink += out.len();
                        }
                        assert_eq!(sink, total);
                        td.push(cpu() - s);
                    }
                    // Round trip equals the original, through the filter when used.
                    for (cb, rb) in comp.iter().zip(&raw) {
                        let mut out = c.decompress(cb, rb.len()).expect("round trip");
                        if shuffled {
                            out = unshuffle8(&out);
                        }
                        assert_eq!(&out, rb, "{} round trip", c.name());
                    }
                    println!(
                        "| {} | {} | {} | {:.3} | {:.0} | {:.0} |",
                        c.name(),
                        if shuffled { "shuffle8" } else { "none" },
                        cbytes,
                        cbytes as f64 / total as f64,
                        mb / median(t),
                        mb / median(td)
                    );
                }
            }
        }
        "checksum" => {
            println!("| checksum | MB/s over the blocks (native) | ns per block |");
            println!("|---|---|---|");
            for k in CHECKSUMS {
                let mut t = vec![];
                let mut acc = 0u64;
                for _ in 0..5 {
                    let s = cpu();
                    for b in &raw {
                        acc = acc.wrapping_add(checksum(k, b));
                    }
                    t.push(cpu() - s);
                }
                let m = median(t);
                println!("| {k} | {:.0} | {:.0} |", mb / m, m * 1_000_000_000.0 / raw.len() as f64);
                std::hint::black_box(acc);
            }
        }
        "dump" => {
            let dir = &a[3];
            let lens: Vec<u8> = raw.iter().flat_map(|b| (b.len() as u32).to_le_bytes()).collect();
            let pack = |blocks: &[Vec<u8>]| -> Vec<u8> {
                let mut offs = vec![0u32];
                for b in blocks {
                    offs.push(offs.last().unwrap() + b.len() as u32);
                }
                let mut out = (blocks.len() as u32).to_le_bytes().to_vec();
                out.extend(offs.iter().flat_map(|o| o.to_le_bytes()));
                for b in blocks {
                    out.extend(b);
                }
                out
            };
            std::fs::write(format!("{dir}/{fixture}.lens"), &lens).unwrap();
            let rawcat: Vec<u8> = raw.iter().flatten().copied().collect();
            std::fs::write(format!("{dir}/{fixture}.raw"), &rawcat).unwrap();
            let sh: Vec<Vec<u8>> = raw.iter().map(|b| shuffle8(b)).collect();
            for (name, c, input) in [
                ("deflate1", Codec::Deflate1, &raw),
                ("deflate1-shuf", Codec::Deflate1, &sh),
                ("lz4", Codec::Lz4, &raw),
                ("lz4-shuf", Codec::Lz4, &sh),
            ] {
                let comp: Vec<Vec<u8>> = input.iter().map(|b| c.compress(b)).collect();
                std::fs::write(format!("{dir}/{fixture}.{name}"), pack(&comp)).unwrap();
            }
            println!("dumped {} blocks to {dir}", raw.len());
        }
        _ => eprintln!("see the header of src/main.rs"),
    }
}
