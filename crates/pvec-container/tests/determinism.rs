//! The writer's one job (issue P4.4): the same input gives the same bytes, whatever the order it
//! arrives in, the thread that pushes it, the process it runs in, the machine, or the time.

mod support;

use pvec_container::writer::SortKey;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use support::*;

fn write(input: &Input) -> Vec<u8> {
    match input.write() {
        Ok(bytes) => bytes,
        Err(e) => panic!("{}: the writer refused a valid fixture: {e}", input.name),
    }
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/digests.txt")
}

fn spline_heavy_is_in_the_set() -> bool {
    fixtures().iter().any(|f| f.name == "spline-heavy")
}

#[test]
fn there_are_at_least_three_fixtures_and_one_is_spline_heavy() {
    assert!(fixtures().len() >= 3);
    assert!(spline_heavy_is_in_the_set());
    let splines = spline_heavy().entities().filter(|e| matches!(e, Ent::Spline { .. })).count();
    assert!(splines >= 200, "the spline-heavy fixture has {splines} splines");
}

#[test]
fn the_same_input_written_twice_is_byte_identical_on_every_fixture() {
    for fx in fixtures() {
        let (a, b) = (write(&fx), write(&fx));
        assert!(!a.is_empty(), "{}: empty output", fx.name);
        assert_eq!(a, b, "{}: two writes of one input differ", fx.name);
        assert_eq!(sha256_hex(&a), sha256_hex(&b));
    }
}

#[test]
fn every_fixture_passes_the_reader_and_the_format_gate() {
    for fx in fixtures() {
        let bytes = write(&fx);
        match verify(&bytes, &fx) {
            Ok(s) => assert!(s.blocks >= 1 && s.entities >= 1, "{}: nothing written", fx.name),
            Err(e) => panic!("{}: {e}", fx.name),
        }
    }
}

#[test]
fn shuffling_the_entities_streams_and_capabilities_gives_the_same_bytes() {
    for fx in fixtures() {
        let reference = write(&fx);
        for seed in 1..=24 {
            let shuffled = fx.shuffled(seed);
            if fx.items.len() > 2 {
                assert!(
                    shuffled.items.iter().zip(&fx.items).any(|(a, b)| format!("{a:?}") != format!("{b:?}")),
                    "seed {seed} did not reorder {}",
                    fx.name
                );
            }
            assert_eq!(write(&shuffled), reference, "{}: seed {seed} changed the bytes", fx.name);
        }
    }
}

#[test]
fn a_reversed_input_gives_the_same_bytes() {
    for fx in fixtures() {
        let mut reversed = fx.clone();
        reversed.items.reverse();
        reversed.config.capabilities.reverse();
        assert_eq!(write(&reversed), write(&fx), "{}", fx.name);
    }
}

fn pushed_concurrently(input: &Input, threads: usize) -> Vec<u8> {
    let writer = Mutex::new(Some(pvec_container::writer::ContainerWriter::new(input.config.clone())));
    std::thread::scope(|scope| {
        for t in 0..threads {
            let writer = &writer;
            scope.spawn(move || {
                for item in input.items.iter().skip(t).step_by(threads) {
                    let mut guard = writer.lock().unwrap();
                    let w = guard.as_mut().unwrap();
                    match item {
                        Item::Ent(e) => e.push(w).unwrap(),
                        Item::Stream(s) => w.push_stream(s.clone()).unwrap(),
                    }
                }
            });
        }
    });
    let w = writer.lock().unwrap().take().unwrap();
    w.finish().unwrap()
}

#[test]
fn the_number_of_threads_that_feed_the_writer_does_not_change_the_bytes() {
    for fx in fixtures() {
        let reference = write(&fx);
        for threads in [1, 2, 3, 4, 8] {
            for round in 0..4 {
                let bytes = pushed_concurrently(&fx, threads);
                assert_eq!(bytes, reference, "{}: {threads} threads, round {round}", fx.name);
            }
        }
    }
}

#[test]
fn writes_running_at_the_same_time_on_many_threads_agree() {
    for fx in fixtures() {
        let reference = write(&fx);
        let results: Vec<Vec<u8>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| write(&fx))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(results.iter().all(|r| *r == reference), "{}", fx.name);
    }
}

#[test]
fn the_wall_clock_does_not_reach_the_output() {
    let fx = plan();
    let first = write(&fx);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(write(&fx), first, "the output depends on the time of day");
}

/// Run by the next test in a child process, which sets `PVEC_DETERMINISM_CHILD`; otherwise it does
/// nothing.
#[test]
fn child_prints_digests() {
    if std::env::var_os("PVEC_DETERMINISM_CHILD").is_none() {
        return;
    }
    for fx in fixtures() {
        println!("DIGEST {} {}", fx.name, sha256_hex(&write(&fx)));
    }
}

fn digests_from_child(dir: Option<&std::path::Path>, env: &[(&str, &str)]) -> BTreeMap<String, String> {
    let exe = std::env::current_exe().unwrap();
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", "child_prints_digests", "--nocapture", "--test-threads=1"])
        .env("PVEC_DETERMINISM_CHILD", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd.output().expect("the child process starts");
    assert!(out.status.success(), "the child failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout)
        .lines()
        // libtest prints "test name ... " in front of the first line, so look for the marker, not a prefix
        .filter_map(|l| l.split_once("DIGEST ").map(|(_, rest)| rest))
        .filter_map(|l| l.split_once(' '))
        .map(|(n, d)| (n.to_string(), d.to_string()))
        .collect()
}

fn digests_here() -> BTreeMap<String, String> {
    fixtures().iter().map(|f| (f.name.to_string(), sha256_hex(&write(f)))).collect()
}

#[test]
fn separate_processes_write_the_same_digests() {
    let here = digests_here();
    assert_eq!(here.len(), fixtures().len());
    for run in 0..2 {
        let child = digests_from_child(None, &[]);
        assert_eq!(child, here, "child process {run} wrote different bytes");
    }
}

#[test]
fn another_directory_environment_time_zone_and_hostname_change_nothing() {
    let here = digests_here();
    let other = std::env::temp_dir();
    let child = digests_from_child(
        Some(&other),
        &[
            ("TZ", "Pacific/Kiritimati"),
            ("HOSTNAME", "some-other-host"),
            ("HOME", "/nonexistent/home"),
            ("USER", "someone-else"),
            ("LANG", "tr_TR.UTF-8"),
            ("PWD", "/nonexistent/pwd"),
        ],
    );
    assert_eq!(child, here);
}

#[test]
fn the_digests_match_the_committed_golden_file_so_every_architecture_agrees() {
    let here = digests_here();
    let text: String = here.iter().map(|(n, d)| format!("{n} {d}\n")).collect();
    if std::env::var_os("PVEC_BLESS_DIGESTS").is_some() {
        fs::create_dir_all(golden_path().parent().unwrap()).unwrap();
        fs::write(golden_path(), &text).unwrap();
    }
    let committed = fs::read_to_string(golden_path()).unwrap_or_default();
    assert!(
        committed == text,
        "tests/golden/digests.txt differs from what this machine ({}) writes. A change of format is blessed \
         with PVEC_BLESS_DIGESTS=1; a difference between architectures is a bug.\n--- committed\n{committed}--- here\n{text}",
        std::env::consts::ARCH
    );
}

/// A shuffle must be able to tell that a sort is gone: with the sort switched off, some reordering of
/// the input changes the bytes (or the write fails).
fn the_suite_notices_without(key: SortKey) {
    let base = mixed();
    let reference = base.write().expect("the unmutated writer accepts the fixture");
    for seed in 1..=24 {
        assert_eq!(base.shuffled(seed).write().unwrap(), reference, "the unmutated writer is not stable");
    }
    let mutated = base.write_skipping(key);
    let noticed = (1..=24).any(|seed| match (&mutated, base.shuffled(seed).write_skipping(key)) {
        (Ok(m), Ok(s)) => *m != s,
        _ => true,
    });
    assert!(
        noticed,
        "without the {key:?} sort no shuffle changed the output: the suite does not need that sort"
    );
}

#[test]
fn removing_the_block_kind_sort_turns_the_suite_red() {
    the_suite_notices_without(SortKey::BlockKind);
}

#[test]
fn removing_the_spatial_cell_sort_turns_the_suite_red() {
    the_suite_notices_without(SortKey::SpatialCell);
}

#[test]
fn removing_the_geometry_type_sort_turns_the_suite_red() {
    the_suite_notices_without(SortKey::GeometryType);
}

#[test]
fn removing_the_entity_id_sort_turns_the_suite_red() {
    the_suite_notices_without(SortKey::EntityId);
}

#[test]
fn removing_the_capability_id_sort_turns_the_suite_red() {
    the_suite_notices_without(SortKey::CapabilityId);
}

#[test]
fn removing_the_stream_id_sort_turns_the_suite_red() {
    the_suite_notices_without(SortKey::StreamId);
}

#[test]
fn every_sort_key_has_a_mutation_test() {
    assert_eq!(SortKey::ALL.len(), 6);
}

// ---- no clock, path, hash order or address reaches the output ----

/// The code of a source file: line comments dropped, and everything from `#[cfg(test)]` on.
fn code_of(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        out.push_str(line.split("//").next().unwrap_or(""));
        out.push('\n');
    }
    out
}

const AMBIENT: [&str; 22] = [
    "SystemTime",
    "Instant::",
    "UNIX_EPOCH",
    "std::env",
    "std::fs",
    "std::path",
    "std::process",
    "std::thread",
    "std::net",
    "RandomState",
    "DefaultHasher",
    "thread_rng",
    "getrandom",
    "current_dir",
    "hostname",
    "{:p}",
    "as *const",
    "as *mut",
    "expose_provenance",
    ".addr()",
    "as usize as",
    "from_ne_bytes_of_pointer",
];

fn ambient_inputs(code: &str) -> Vec<&'static str> {
    AMBIENT.iter().copied().filter(|t| code.contains(t)).collect()
}

/// Names declared as a `HashMap` or `HashSet` that are then iterated: the order is the hasher's.
fn hash_iterations(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in code.lines() {
        if line.contains("HashMap<")
            || line.contains("HashSet<")
            || line.contains("HashMap::")
            || line.contains("HashSet::")
        {
            let head = line.split([':', '=']).next().unwrap_or("");
            if let Some(name) = head.split_whitespace().last() {
                let name = name.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
                if !name.is_empty() && name != "use" && name != "std" {
                    names.push(name.to_string());
                }
            }
        }
    }
    let mut hits = Vec::new();
    for name in names {
        for method in [".iter(", ".keys(", ".values(", ".into_iter(", ".drain(", ".iter_mut(", ".values_mut("]
        {
            if code.contains(&format!("{name}{method}")) {
                hits.push(format!("{name}{method}"));
            }
        }
        for form in [format!(" in {name} "), format!(" in &{name} "), format!(" in &mut {name} ")] {
            if code.contains(&form) {
                hits.push(form.trim().to_string());
            }
        }
    }
    hits
}

#[test]
fn the_source_scanner_flags_what_it_should() {
    for bad in [
        "let t = std::time::SystemTime::now();",
        "let t = Instant::now();",
        "let d = std::env::var(\"HOME\");",
        "let p = std::path::PathBuf::new();",
        "format!(\"{:p}\", &x)",
        "let a = &x as *const u8;",
        "let s = RandomState::new();",
    ] {
        assert!(!ambient_inputs(bad).is_empty(), "missed: {bad}");
    }
    assert!(ambient_inputs("let x = 1u32.to_le_bytes();").is_empty());
    let looped = "let mut seen: HashMap<u32, u32> = HashMap::new();\nfor (k, v) in seen.iter() {}\n";
    assert!(!hash_iterations(looped).is_empty());
    let looked_up =
        "let mut seen: HashMap<u32, u32> = HashMap::new();\nseen.insert(1, 2);\nlet _ = seen.get(&1);\n";
    assert!(hash_iterations(looked_up).is_empty());
    assert_eq!(code_of("a // SystemTime\n#[cfg(test)]\nSystemTime"), "a \n");
}

#[test]
fn no_clock_path_environment_hash_order_or_address_is_read_by_the_crate() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files =
        vec![src.join("writer.rs"), src.join("directory.rs"), src.join("header.rs"), src.join("manifest.rs")];
    for entry in fs::read_dir(src.join("blocks")).unwrap().flatten() {
        files.push(entry.path());
    }
    assert!(src.join("writer.rs").exists());
    for f in files {
        let code = code_of(&fs::read_to_string(&f).unwrap());
        // aligned.rs reads the address of its own buffer to align a cast; it never writes one.
        let found: Vec<_> = ambient_inputs(&code)
            .into_iter()
            .filter(|t| !f.ends_with("aligned.rs") || *t != ".addr()")
            .collect();
        assert!(found.is_empty(), "{}: reads ambient input {found:?}", f.display());
        assert!(hash_iterations(&code).is_empty(), "{}: iterates a hash collection", f.display());
    }
}

#[test]
fn the_writer_source_exists_and_is_where_the_checklist_says() {
    let text = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/writer.rs")).unwrap();
    assert!(text.contains("pub fn finish("), "the writer has no finish");
    let doc = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/CONTAINER.md"))
        .unwrap();
    assert!(doc.contains("### 4.6 Review checklist"), "docs/CONTAINER.md has no review checklist entry");
    for needle in ["wall-clock", "absolute path", "hash-map iteration", "pointer", "hostname"] {
        assert!(doc.contains(needle), "the checklist does not mention {needle}");
    }
}
