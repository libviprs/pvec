//! Issue #158: a long-lived helper inside the hermetic container, reached through the pipes of one
//! `docker exec -i`, so the container keeps `--network none` and a test can drive a container-side
//! server. A fake `docker` found through an explicit search path stands in for the real one.
#![cfg(unix)]

use pvec_hermetic::{Config, Container};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const IMAGE: &str = "registry.invalid/pvec/range-source@sha256:0000000000000000000000000000000000000000000000000000000000000000";

static N: AtomicUsize = AtomicUsize::new(0);

struct Fake {
    dir: PathBuf,
}

impl Fake {
    /// `exec` echoes its standard input back, line by line, as a helper would answer.
    fn new() -> Fake {
        let dir = std::env::temp_dir().join(format!(
            "pvec-fake-docker-158-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> \"{log}\"\n/usr/bin/env > \"{env}\"\ncase \"$1\" in\n  exec) exec /bin/cat;;\nesac\nexit 0\n",
            log = dir.join("calls.log").display(),
            env = dir.join("env.log").display(),
        );
        let bin = dir.join("docker");
        fs::write(&bin, script).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        Fake { dir }
    }

    fn config(&self) -> Config {
        Config {
            image: IMAGE.to_string(),
            mount: self.dir.clone(),
            search_path: Some(self.dir.clone().into()),
        }
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        // The test's own scratch, under the system temp directory, named in full.
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn exec_stream_pipes_standard_input_and_output_of_one_interactive_exec() {
    let f = Fake::new();
    let c = Container::start_with_command(&f.config(), &["sleep", "600"]).unwrap();
    let mut child = c.exec_stream(&["python3", "/corpus/range_server.py"]).expect("a child");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut out = BufReader::new(child.stdout.take().expect("stdout is piped"));
    for line in ["R 0 16", "R 4096 8"] {
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let mut back = String::new();
        out.read_line(&mut back).unwrap();
        assert_eq!(back.trim_end(), line);
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
    let exec: Vec<String> = f.calls().into_iter().filter(|c| c.starts_with("exec")).collect();
    assert_eq!(exec.len(), 1, "{exec:?}");
    assert!(
        exec[0].starts_with(&format!("exec -i {} python3 /corpus/range_server.py", c.name())),
        "{exec:?}"
    );
    c.stop().unwrap();
}

#[test]
fn exec_stream_keeps_the_network_off_and_forwards_no_credential() {
    let f = Fake::new();
    let c = Container::start_with_command(&f.config(), &["sleep", "600"]).unwrap();
    let mut child = c.exec_stream(&["true"]).expect("a child");
    drop(child.stdin.take());
    child.wait().unwrap();
    let run = f.calls().into_iter().find(|c| c.starts_with("run")).expect("a run");
    assert!(run.contains("--network none"), "{run}");
    assert!(!run.contains(" -p ") && !run.contains(" -e "), "{run}");
    let env = fs::read_to_string(f.dir.join("env.log")).unwrap();
    assert!(!env.contains("TOKEN") && !env.contains("SECRET"), "{env}");
    c.stop().unwrap();
}

#[test]
fn a_container_with_a_streaming_exec_is_still_removed_when_the_test_panics() {
    let f = Fake::new();
    let started = std::sync::atomic::AtomicBool::new(false);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let c = Container::start_with_command(&f.config(), &["sleep", "600"]).unwrap();
        let child = c.exec_stream(&["cat"]);
        started.store(child.is_ok(), Ordering::SeqCst);
        panic!("the test failed");
    }));
    assert!(result.is_err());
    assert!(started.load(Ordering::SeqCst), "the streaming exec never started");
    assert!(f.calls().iter().any(|c| c.starts_with("rm -f pvec-hermetic-")), "{:?}", f.calls());
}
