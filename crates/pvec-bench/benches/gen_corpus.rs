//! The corpus generator and its lock, as a command.
//!
//! ```text
//! cargo bench -p pvec-bench --bench gen_corpus -- write             rewrite corpus.lock.json and viewports.csv
//! cargo bench -p pvec-bench --bench gen_corpus -- measure NAME      build NAME, print one measured.csv row
//! cargo bench -p pvec-bench --bench gen_corpus -- check             rebuild every stored entry and compare
//! ```
//!
//! `measure` reads the host name from `CORPUS_HOST` and the peak resident set from `/proc/self/status`
//! (Linux only; elsewhere it prints 0 and the lock refuses the row).

use pvec_bench::{Spec, build, lock};
use pvec_corpus::json::{self, Value};
use pvec_corpus::sha256::hex_digest;
use std::path::PathBuf;
use std::time::Instant;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/corpus")
}

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    match args.first().map(String::as_str) {
        Some("write") => {
            let measured = std::fs::read_to_string(corpus_dir().join("measured.csv")).unwrap_or_default();
            let (json, csv) = lock::render(&measured).unwrap_or_else(|e| panic!("{e}"));
            std::fs::write(corpus_dir().join("corpus.lock.json"), json).expect("lock");
            std::fs::write(corpus_dir().join("viewports.csv"), csv).expect("viewports");
        }
        Some("measure") => {
            let name = args.get(1).expect("an entry name");
            let spec = Spec::named(name).expect("a generated entry");
            let start = Instant::now();
            let bytes = spec.artifact().unwrap_or_else(|e| panic!("{e}"));
            let millis = start.elapsed().as_millis();
            println!(
                "{name},{}-{},{},{millis},{},{},{}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::env::var("CORPUS_HOST").unwrap_or_else(|_| "unnamed".into()),
                peak_rss_kib(),
                hex_digest(&bytes),
                bytes.len()
            );
        }
        Some("check") => {
            let text = std::fs::read_to_string(corpus_dir().join("corpus.lock.json")).expect("lock");
            let lock = json::parse(text.as_bytes()).expect("json");
            let mut bad = 0;
            for e in lock.get("entries").and_then(Value::as_arr).expect("entries") {
                let name = e.get("name").and_then(Value::as_str).expect("name");
                if e.get("on_demand_only").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let want = e.get("sha256").and_then(Value::as_str).expect("sha");
                let got = hex_digest(&build(name).expect("builds"));
                println!("{} {name}", if got == want { "ok  " } else { "FAIL" });
                bad += usize::from(got != want);
            }
            assert_eq!(bad, 0, "entries whose digest differs from the lock");
        }
        _ => eprintln!("usage: gen_corpus write | measure NAME | check"),
    }
}
