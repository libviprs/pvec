//! Issue #21: the hermetic fixture can start an image with its own command and run commands inside the
//! container it started, which the GML schema validation uses. A fake `docker` found through an explicit
//! search path stands in for the real one, so nothing here needs Docker.
#![cfg(unix)]

use pvec_hermetic::{Config, Container};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

const IMAGE: &str =
    "registry.invalid/pvec/validator@sha256:0000000000000000000000000000000000000000000000000000000000000000";

static N: AtomicUsize = AtomicUsize::new(0);

struct Fake {
    dir: PathBuf,
}

impl Fake {
    fn new() -> Fake {
        let dir = std::env::temp_dir().join(format!(
            "pvec-fake-docker-21-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> \"{log}\"\ncase \"$1\" in\n  exec) echo \"out:$*\"; echo \"err:$*\" >&2; [ \"$3\" = fail ] && exit 3; exit 0;;\nesac\nexit 0\n",
            log = dir.join("calls.log").display()
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
        fs::read_to_string(self.dir.join("calls.log")).unwrap().lines().map(str::to_string).collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_command_given_to_start_follows_the_image_on_the_run_line() {
    let f = Fake::new();
    let c = Container::start_with_command(&f.config(), &["sleep", "900"]).unwrap();
    let run = f.calls().into_iter().find(|l| l.starts_with("run ")).unwrap();
    assert!(run.ends_with(&format!("{IMAGE} sleep 900")), "{run}");
    assert!(run.contains("--network none") && run.contains("--read-only"), "{run}");
    c.stop().unwrap();
}

#[test]
fn start_without_a_command_is_unchanged() {
    let f = Fake::new();
    let c = Container::start(&f.config()).unwrap();
    let run = f.calls().into_iter().find(|l| l.starts_with("run ")).unwrap();
    assert!(run.ends_with(IMAGE), "{run}");
    c.stop().unwrap();
}

#[test]
fn exec_runs_in_the_named_container_and_reports_status_and_output() {
    let f = Fake::new();
    let c = Container::start_with_command(&f.config(), &["sleep", "900"]).unwrap();
    let ok = c.exec(&["echo", "hi"]).unwrap();
    assert!(ok.success);
    assert_eq!(ok.stdout.trim(), format!("out:exec {} echo hi", c.name()));
    assert_eq!(ok.stderr.trim(), format!("err:exec {} echo hi", c.name()));
    let bad = c.exec(&["fail"]).unwrap();
    assert!(!bad.success, "a command that exits non-zero is reported, not turned into an error");
    c.stop().unwrap();
}

#[test]
fn the_container_is_still_removed_after_an_exec() {
    let f = Fake::new();
    {
        let c = Container::start_with_command(&f.config(), &["sleep", "900"]).unwrap();
        c.exec(&["true"]).unwrap();
    }
    assert!(f.calls().iter().any(|l| l.starts_with("rm -f pvec-hermetic-")), "{:?}", f.calls());
}
