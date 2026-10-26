//! Issue P8.1 (pvec#60): the benchmark corpus is fixed, named, pinned and checkable. Each test is named
//! for the acceptance criterion it holds.

use pvec_bench::{CLASSES, bounds, build, counts, viewports};
use pvec_corpus::json::{self, Value};
use pvec_corpus::sha256::hex_digest;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const FIXTURE_COMMIT: &str = "f94f7f22b7aaff36eaaf6d5501fafc42054f22f2";
const BUNDLE_SHA256: &str = "901370036bbffbbb8cb30471c2bf90f171b4f966c5bdfea6b140591cd840ca50";
const QUARANTINED: [&str; 3] = ["general-affine-001", "shear-001", "singular-001"];
/// Entries up to this size are rebuilt by the default test run; the bigger ones by the ignored test.
const FAST: i64 = 100_000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(name: &str) -> String {
    fs::read_to_string(root().join("benches/corpus").join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn lock() -> Value {
    json::parse(read("corpus.lock.json").as_bytes()).expect("the lock parses as strict JSON")
}

fn entries(lock: &Value) -> Vec<&Value> {
    lock.get("entries").and_then(Value::as_arr).expect("entries").iter().collect()
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_else(|| panic!("no string `{key}` in {v:?}"))
}

fn n(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_int).unwrap_or_else(|| panic!("no integer `{key}`"))
}

fn hex_f64(h: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(h.trim_start_matches("0x"), 16).expect("hex bits"))
}

fn lock_bounds(e: &Value) -> [f64; 4] {
    let b = e.get("bounds").and_then(Value::as_arr).expect("bounds");
    assert_eq!(b.len(), 4, "{}: four bounds", s(e, "name"));
    [0, 1, 2, 3].map(|i| hex_f64(b[i].as_str().expect("hex")))
}

fn on_demand(e: &Value) -> bool {
    e.get("on_demand_only").and_then(Value::as_bool) == Some(true)
}

#[test]
fn the_lock_lists_every_entry_with_a_digest_counts_and_an_evidence_class() {
    let lock = lock();
    let all = entries(&lock);
    assert!(all.len() >= 20, "{} entries", all.len());
    let mut names = BTreeSet::new();
    for e in all {
        let name = s(e, "name");
        assert!(names.insert(name), "{name} twice");
        let sha = s(e, "sha256");
        assert!(sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert!(n(e, "bytes") > 0 && n(e, "entities") > 0, "{name}");
        let by_capability = e.get("counts").and_then(Value::as_obj).expect("counts");
        let total: i64 = by_capability.iter().map(|(_, v)| v.as_int().expect("count")).sum();
        assert_eq!(total, n(e, "entities"), "{name}: the capability counts add up to the entities");
        assert!(e.get("evidence_class").is_some(), "{name}: evidence_class is stated (a string or null)");
        assert!(!s(e, "licence").is_empty() && !s(e, "redistribution").is_empty(), "{name}");
        assert!(["generator", "vendored-bundle"].contains(&s(e, "obtain")), "{name}");
    }
}

#[test]
fn every_digest_in_the_lock_matches_the_rebuilt_artifact() {
    let lock = lock();
    for e in entries(&lock).into_iter().filter(|e| n(e, "entities") <= FAST) {
        let name = s(e, "name");
        let bytes = build(name).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(hex_digest(&bytes), s(e, "sha256"), "{name}: digest");
        assert_eq!(bytes.len() as i64, n(e, "bytes"), "{name}: size");
        assert_eq!(lock_bounds(e), bounds(&bytes).expect("bounds"), "{name}: source bounds");
        let live = counts(&bytes).expect("counts");
        let recorded = e.get("counts").and_then(Value::as_obj).expect("counts");
        assert_eq!(live.len(), recorded.len(), "{name}: capabilities");
        for (cap, v) in recorded {
            assert_eq!(live.get(cap).copied(), v.as_int().map(|i| i as u64), "{name}: {cap}");
        }
    }
}

#[test]
#[ignore = "rebuilds the 10^6 entries; run it on both architectures (release)"]
fn every_large_digest_in_the_lock_matches_the_rebuilt_artifact() {
    let lock = lock();
    let big: Vec<_> =
        entries(&lock).into_iter().filter(|e| n(e, "entities") > FAST && !on_demand(e)).collect();
    assert!(!big.is_empty(), "the 10^6 rungs are in the lock");
    for e in big {
        let bytes = build(s(e, "name")).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(hex_digest(&bytes), s(e, "sha256"), "{}", s(e, "name"));
    }
}

#[test]
fn all_seven_classes_are_populated_and_the_corpus_document_names_each_with_its_counts() {
    let lock = lock();
    let doc = read("CORPUS.md");
    for class in CLASSES {
        let members: Vec<_> = entries(&lock).into_iter().filter(|e| s(e, "class") == class).collect();
        assert!(!members.is_empty(), "class {class} has no member");
        assert!(doc.contains(class), "CORPUS.md never names {class}");
        for m in members {
            assert!(n(m, "entities") > 0);
            assert!(doc.contains(s(m, "name")), "CORPUS.md never names the entry {}", s(m, "name"));
        }
    }
}

#[test]
fn the_scale_ladder_has_1e3_1e5_1e6_and_a_1e7_case_generated_on_demand() {
    let lock = lock();
    let at = |entities: i64| -> Vec<&Value> {
        entries(&lock)
            .into_iter()
            .filter(|e| s(e, "class") == "architectural" && n(e, "entities") == entities)
            .collect()
    };
    for rung in [1_000, 100_000, 1_000_000] {
        assert_eq!(at(rung).len(), 1, "one architectural entry of {rung} entities");
    }
    let huge = at(10_000_000);
    assert_eq!(huge.len(), 1, "one 10^7 entry");
    let e = huge[0];
    assert!(on_demand(e), "the 10^7 case is never stored");
    let platforms = e.get("generation").and_then(Value::as_arr).expect("generation records");
    assert!(!platforms.is_empty());
    for g in platforms {
        assert!(n(g, "peak_rss_kib") > 0 && n(g, "millis") > 0, "time and peak memory are recorded");
        assert_eq!(s(g, "sha256"), s(e, "sha256"), "{}: the same bytes on every platform", s(g, "platform"));
    }
}

#[test]
fn the_fixture_revision_is_pinned_and_a_real_world_entry_is_never_promoted() {
    let lock = lock();
    let rev = lock.get("fixture_revision").expect("fixture_revision");
    assert_eq!(s(rev, "repo"), "libviprs/pvec-gold-fixtures");
    assert_eq!(s(rev, "epic"), "libviprs/pvec-gold-fixtures#1");
    assert_eq!(s(rev, "commit"), FIXTURE_COMMIT);
    assert_eq!(s(rev, "bundle_sha256"), BUNDLE_SHA256);
    let doc = read("CORPUS.md");
    assert!(doc.contains(FIXTURE_COMMIT) && doc.contains(BUNDLE_SHA256));
    for e in entries(&lock) {
        let class = e.get("evidence_class").and_then(Value::as_str);
        match s(e, "obtain") {
            "vendored-bundle" => assert_eq!(class, Some("KNOWN_TRUTH_EXACT"), "{}", s(e, "name")),
            _ => assert_eq!(class, None, "{}: a generated entry is no EPIC O evidence", s(e, "name")),
        }
    }
}

#[test]
fn quarantined_and_unobtainable_entries_are_excluded_from_baselines_and_listed_with_a_reason() {
    let lock = lock();
    let ex = lock.get("exclusions").and_then(Value::as_arr).expect("exclusions");
    for q in QUARANTINED {
        let row = ex.iter().find(|r| s(r, "name") == q).unwrap_or_else(|| panic!("{q} is not listed"));
        assert_eq!(s(row, "kind"), "QUARANTINED");
        assert!(!s(row, "reason").is_empty());
    }
    let listed: BTreeSet<&str> = ex.iter().map(|r| s(r, "name")).collect();
    for e in entries(&lock) {
        assert!(!listed.contains(s(e, "name")), "{} is both an entry and an exclusion", s(e, "name"));
        assert_eq!(e.get("baseline_eligible").and_then(Value::as_bool), Some(true));
    }
    let doc = read("CORPUS.md");
    for name in &listed {
        assert!(doc.contains(name), "CORPUS.md does not list the exclusion {name}");
    }
}

#[test]
fn the_recorded_viewport_set_is_checked_in_and_is_200_per_entry() {
    let lock = lock();
    let csv = read("viewports.csv");
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("entry,index,x0,y0,x1,y1"));
    for e in entries(&lock) {
        let name = s(e, "name");
        let want = viewports(name, lock_bounds(e));
        assert_eq!(want.len(), 200, "{name}");
        for (i, v) in want.iter().enumerate() {
            let line = lines.next().unwrap_or_else(|| panic!("viewports.csv ends inside {name}"));
            let f: Vec<&str> = line.split(',').collect();
            assert_eq!((f[0], f[1].parse::<usize>().ok()), (name, Some(i)), "{line}");
            let got: Vec<u64> = f[2..].iter().map(|x| x.parse::<f64>().expect("number").to_bits()).collect();
            assert_eq!(got, v.map(f64::to_bits).to_vec(), "{line}");
        }
    }
    assert_eq!(lines.next(), None, "nothing after the last entry");
}

#[test]
fn generation_is_deterministic_and_the_document_states_both_platforms() {
    let lock = lock();
    let small = entries(&lock).into_iter().find(|e| n(e, "entities") == 1_000).expect("a 10^3 entry");
    let name = s(small, "name");
    assert_eq!(build(name), build(name), "the same seed gives the same bytes");
    let doc = read("CORPUS.md");
    for needle in ["arm64", "native x86_64", "bit for bit", "peak memory", "no proprietary converter"] {
        assert!(doc.contains(needle), "CORPUS.md never says {needle:?}");
    }
}

#[test]
fn the_corpus_half_starts_no_process_and_opens_no_socket() {
    let mut files = vec![root().join("benches/gen_corpus.rs")];
    for f in fs::read_dir(root().join("src")).expect("src") {
        files.push(f.expect("entry").path());
    }
    for file in files {
        let text = fs::read_to_string(&file).expect("readable");
        for banned in ["std::process", "Command::new", "TcpStream", "UdpSocket", "docker"] {
            assert!(!text.contains(banned), "{} mentions {banned}", file.display());
        }
    }
}
