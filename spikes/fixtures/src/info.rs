//! Prints the corpus revision and, per fixture, the counts and content hash that tables cite.
//! `fixture-info --p21 <dir>` also writes each Part 21 baseline and prints its size.

use pvec_fixtures::{REVISION, generate, p21};
use std::io::BufWriter;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let p21_dir = args.iter().position(|a| a == "--p21").and_then(|i| args.get(i + 1)).cloned();
    println!("corpus revision: {REVISION}\n");
    println!(
        "| fixture | entities | extent | lines | circles | arcs | ellipses | polylines | splines | composites | content hash (FNV-1a) | Part 21 bytes |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    for d in generate::all() {
        let c = d.kind_counts();
        let bytes = match &p21_dir {
            Some(dir) => {
                let f = std::fs::File::create(format!("{dir}/{}.stp", d.name)).expect("create");
                let mut w = BufWriter::new(f);
                p21::write_part21(&d, &mut w).expect("write").to_string()
            }
            None => "-".to_string(),
        };
        println!(
            "| {} | {} | {}x{} | {} | {} | {} | {} | {} | {} | {} | {:016x} | {} |",
            d.name,
            d.entities.len(),
            d.extent[2],
            d.extent[3],
            c[0],
            c[1],
            c[2],
            c[3],
            c[4],
            c[5],
            c[6],
            d.checksum(),
            bytes
        );
    }
}
