//! Writes one large raster `.pvec` from a seed and prints its size and SHA-256:
//!
//! ```text
//! cargo run --release -p pvec-testkit --example emit_large_raster -- <out> \
//!     [--seed S] [--width W] [--height H] [--tile-edge E] [--bit-depth 1|8] [--lines N] [--curves N]
//! ```
//!
//! Defaults: seed 1, 8192 by 8192, tiles of 1024, 8 bits gray, 400 lines, 40 curves. The same arguments
//! give the same bytes. Tile edge is 256, 512 or 1024.

use pvec_testkit::large_raster::large_raster;
use pvec_testkit::tiler::Shape;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().filter(|a| !a.starts_with("--")).unwrap_or_else(|| usage("missing <out>"));
    let (mut seed, mut shape, mut lines, mut curves) =
        (1u64, Shape { width: 8192, height: 8192, tile_edge: 1024, channels: 1, bit_depth: 8 }, 400, 40);
    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_else(|| usage(&format!("{flag} needs a value")));
        let n: u64 = value.parse().unwrap_or_else(|_| usage(&format!("{flag}: {value} is not a number")));
        match flag.as_str() {
            "--seed" => seed = n,
            "--width" => shape.width = n as u32,
            "--height" => shape.height = n as u32,
            "--tile-edge" => shape.tile_edge = n as u32,
            "--bit-depth" => shape.bit_depth = n as u8,
            "--lines" => lines = n as u32,
            "--curves" => curves = n as u32,
            _ => usage(&format!("unknown flag {flag}")),
        }
    }
    if ![256, 512, 1024].contains(&shape.tile_edge) || ![1, 8].contains(&shape.bit_depth) {
        usage("--tile-edge is 256, 512 or 1024 and --bit-depth is 1 or 8");
    }
    if shape.width == 0 || shape.height == 0 {
        usage("--width and --height are at least 1");
    }
    let bytes = large_raster(seed, shape, lines, curves);
    std::fs::write(&out, &bytes).unwrap_or_else(|e| usage(&format!("{out}: {e}")));
    println!("{out} {} bytes sha256 {}", bytes.len(), pvec_corpus::sha256::hex_digest(&bytes));
}

fn usage(why: &str) -> ! {
    eprintln!(
        "{why}\nusage: emit_large_raster <out> [--seed S] [--width W] [--height H] [--tile-edge E] [--bit-depth 1|8] [--lines N] [--curves N]"
    );
    std::process::exit(2)
}
