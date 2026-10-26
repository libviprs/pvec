//! Writes the seed corpus as libFuzzer wants it: `<dir>/<target>/<hash of the input>`, one file per input.
//!
//!     cargo run --release -p pvec-fuzz --bin seed-corpus -- <dir>
//!
//! It also prints the composition, which `docs/FUZZING.md` quotes.

use pvec_fuzz::TARGETS;
use pvec_fuzz::seeds::{DEFERRED, Source, corpus, for_target};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

fn main() {
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: seed-corpus <dir>");
        std::process::exit(64);
    };
    let seeds = corpus();
    for source in [Source::Hostile, Source::KnownTruth, Source::Writer] {
        let of: Vec<_> = seeds.iter().filter(|s| s.source == source).collect();
        let bytes: usize = of.iter().map(|s| s.bytes.len()).sum();
        println!("source {}: {} artifacts, {bytes} bytes", source.name(), of.len());
    }
    for (what, why) in DEFERRED {
        println!("not included: {what}: {why}");
    }
    for (name, _) in TARGETS {
        let target_dir = dir.join(name);
        std::fs::create_dir_all(&target_dir).expect("the corpus directory");
        let inputs = for_target(name, &seeds);
        for input in &inputs {
            let mut h = DefaultHasher::new();
            input.hash(&mut h);
            std::fs::write(target_dir.join(format!("{:016x}", h.finish())), input).expect("a seed file");
        }
        println!("target {name}: {} inputs", inputs.len());
    }
}
