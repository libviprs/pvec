//! Issue #95: the container is started for the run and removed afterwards, on pass and on failure;
//! a Docker-unavailable skip is for developer runs only and CI fails; no credential reaches it.
//! Every test drives a fake `docker` found through an explicit search path, so nothing here needs
//! Docker and nothing touches the process environment.
#![cfg(unix)]

use pvec_hermetic::{Config, Container, ContainerError, Decision, Policy, RunKind};
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const IMAGE: &str = "registry.invalid/pvec/converter@sha256:0000000000000000000000000000000000000000000000000000000000000000";

static N: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory holding a fake `docker` and the log of what it was asked.
struct Fake {
    dir: PathBuf,
}

impl Fake {
    /// `version` and `run` are the shell exit codes of those subcommands, `rm` likewise.
    fn new(version: i32, run: i32, rm: i32) -> Fake {
        let dir = std::env::temp_dir().join(format!("pvec-fake-docker-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir_all(&dir).unwrap();
        let script = format!(
            "#!/bin/sh\n\
             echo \"$@\" >> \"{log}\"\n\
             env | sort > \"{envlog}\"\n\
             case \"$1\" in\n\
               version) exit {version};;\n\
               run) exit {run};;\n\
               rm) exit {rm};;\n\
             esac\n\
             exit 99\n",
            log = dir.join("calls.log").display(),
            envlog = dir.join("env.log").display(),
        );
        let bin = dir.join("docker");
        fs::write(&bin, script).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        Fake { dir }
    }
    fn config(&self) -> Config {
        Config { image: IMAGE.to_string(), mount: self.dir.clone(), search_path: Some(self.dir.clone().into()) }
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
    fn env(&self) -> String {
        fs::read_to_string(self.dir.join("env.log")).unwrap_or_default()
    }
    fn verbs(&self) -> Vec<String> {
        self.calls().iter().map(|c| c.split(' ').next().unwrap().to_string()).collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        // The test's own scratch, under the system temp directory, named in full.
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn empty_dir() -> OsString {
    let d: &Path = Path::new("/nonexistent-pvec-empty-path");
    d.as_os_str().to_os_string()
}

fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_string())
}

// ---- lifecycle -------------------------------------------------------------------------------

#[test]
fn issue_95_a_passing_run_starts_then_removes_the_container() {
    let fake = Fake::new(0, 0, 0);
    let c = Container::start(&fake.config()).unwrap();
    let name = c.name().to_string();
    assert_eq!(fake.verbs(), ["version", "run"]);
    c.stop().unwrap();
    assert_eq!(fake.verbs(), ["version", "run", "rm"]);
    let rm = fake.calls().pop().unwrap();
    assert!(rm.contains("-f") && rm.ends_with(&name), "{rm}");
}

#[test]
fn issue_95_dropping_the_container_removes_it() {
    let fake = Fake::new(0, 0, 0);
    drop(Container::start(&fake.config()).unwrap());
    assert_eq!(fake.verbs(), ["version", "run", "rm"]);
}

#[test]
fn issue_95_a_panicking_test_still_removes_the_container() {
    let fake = Fake::new(0, 0, 0);
    let cfg = fake.config();
    let r = catch_unwind(AssertUnwindSafe(|| {
        let _c = Container::start(&cfg).unwrap();
        panic!("the test body failed");
    }));
    assert!(r.is_err());
    assert_eq!(fake.verbs(), ["version", "run", "rm"], "teardown runs on unwind");
}

#[test]
fn issue_95_an_early_error_return_still_removes_the_container() {
    fn body(cfg: &Config) -> Result<(), String> {
        let _c = Container::start(cfg).map_err(|e| e.to_string())?;
        Err("assertion failed".to_string())
    }
    let fake = Fake::new(0, 0, 0);
    assert!(body(&fake.config()).is_err());
    assert_eq!(fake.verbs(), ["version", "run", "rm"]);
}

#[test]
fn issue_95_each_container_has_its_own_name() {
    let fake = Fake::new(0, 0, 0);
    let a = Container::start(&fake.config()).unwrap();
    let b = Container::start(&fake.config()).unwrap();
    assert_ne!(a.name(), b.name());
    assert!(a.name().starts_with("pvec-hermetic-"));
}

#[test]
fn issue_95_a_failed_removal_is_reported_by_stop() {
    let fake = Fake::new(0, 0, 1);
    let c = Container::start(&fake.config()).unwrap();
    assert!(matches!(c.stop(), Err(ContainerError::TeardownFailed(_))));
}

#[test]
fn issue_95_a_container_that_did_not_start_is_not_removed_again() {
    let fake = Fake::new(0, 1, 0);
    assert!(matches!(Container::start(&fake.config()), Err(ContainerError::StartFailed(_))));
    assert_eq!(fake.verbs(), ["version", "run", "rm"], "a half-created container is cleaned once, by name");
}

// ---- what is started -------------------------------------------------------------------------

#[test]
fn issue_95_an_image_not_pinned_by_digest_is_refused_before_docker_is_called() {
    let fake = Fake::new(0, 0, 0);
    for image in ["alpine", "alpine:3.20", "alpine@sha256:abc", "alpine@sha256:"] {
        let mut cfg = fake.config();
        cfg.image = image.to_string();
        assert!(matches!(Container::start(&cfg), Err(ContainerError::UnpinnedImage(_))), "{image}");
    }
    assert!(fake.calls().is_empty());
}

#[test]
fn issue_95_the_container_has_no_network_no_credentials_and_a_read_only_mount() {
    let fake = Fake::new(0, 0, 0);
    let _c = Container::start(&fake.config()).unwrap();
    let run = fake.calls().into_iter().find(|c| c.starts_with("run ")).unwrap();
    assert!(run.contains("--network none"), "{run}");
    assert!(run.contains("--read-only"), "{run}");
    assert!(run.contains("--rm"), "{run}");
    assert!(run.contains("--platform"), "{run}");
    assert!(run.contains(":/corpus:ro"), "{run}");
    for bad in ["-e ", "--env", "--env-file", "docker.sock", "--privileged", "--volumes-from", "--pid", "--cap-add"] {
        assert!(!run.contains(bad), "{bad} in {run}");
    }
    assert!(run.contains(IMAGE));
}

#[test]
fn issue_95_the_docker_client_sees_no_token_from_the_environment() {
    let fake = Fake::new(0, 0, 0);
    let _c = Container::start(&fake.config()).unwrap();
    let env = fake.env();
    for var in ["GH_TOKEN", "GITHUB_TOKEN", "AWS_", "ANTHROPIC", "SSH_AUTH_SOCK", "HOME="] {
        assert!(!env.contains(var), "{var} reached docker: {env}");
    }
    assert!(env.contains("PATH="), "{env}");
}

// ---- unavailable docker, and the policy ------------------------------------------------------

fn missing() -> Config {
    Config {
        image: IMAGE.to_string(),
        mount: std::env::temp_dir(),
        search_path: Some(empty_dir()),
    }
}

#[test]
fn issue_95_no_docker_on_the_path_is_unavailable() {
    assert!(matches!(Container::start(&missing()), Err(ContainerError::Unavailable(_))));
}

#[test]
fn issue_95_a_daemon_that_does_not_answer_is_unavailable() {
    let fake = Fake::new(1, 0, 0);
    assert!(matches!(Container::start(&fake.config()), Err(ContainerError::Unavailable(_))));
    assert_eq!(fake.verbs(), ["version"], "nothing is run against a dead daemon");
}

#[test]
fn issue_95_a_developer_run_skips_when_docker_is_unavailable() {
    let p = Policy::from_lookup(&env_of(&[("PVEC_DEV_RUN", "1")]));
    assert_eq!(p.kind(), RunKind::Developer);
    let err = Container::start(&missing()).unwrap_err();
    match p.decide(err) {
        Decision::Skip(why) => assert!(why.contains("docker"), "{why}"),
        Decision::Fail(e) => panic!("a developer run skips, got {e}"),
    }
}

#[test]
fn issue_95_a_run_with_nothing_set_fails_rather_than_skips() {
    let p = Policy::from_lookup(&env_of(&[]));
    assert_eq!(p.kind(), RunKind::Strict);
    let err = Container::start(&missing()).unwrap_err();
    assert!(matches!(p.decide(err), Decision::Fail(ContainerError::Unavailable(_))));
}

#[test]
fn issue_95_ci_fails_even_when_the_dev_variable_is_also_set() {
    for ci in ["1", "true", "yes", "anything"] {
        let leaked: &'static [(&'static str, &'static str)] =
            Box::leak(Box::new([("PVEC_DEV_RUN", "1"), ("CI", Box::leak(ci.to_string().into_boxed_str()))]));
        let p = Policy::from_lookup(&env_of(leaked));
        assert_eq!(p.kind(), RunKind::Strict, "CI={ci}");
        let err = Container::start(&missing()).unwrap_err();
        assert!(matches!(p.decide(err), Decision::Fail(_)), "CI={ci}");
    }
}

#[test]
fn issue_95_only_an_explicit_dev_value_marks_a_developer_run() {
    for (v, dev) in [("1", true), ("true", true), ("0", false), ("", false), ("false", false), ("no", false)] {
        let leaked: &'static [(&'static str, &'static str)] =
            Box::leak(Box::new([("PVEC_DEV_RUN", Box::leak(v.to_string().into_boxed_str()) as &'static str)]));
        let p = Policy::from_lookup(&env_of(leaked));
        assert_eq!(p.kind() == RunKind::Developer, dev, "PVEC_DEV_RUN={v:?}");
    }
    // CI=false or CI= does not count as CI.
    let p = Policy::from_lookup(&env_of(&[("PVEC_DEV_RUN", "1"), ("CI", "false")]));
    assert_eq!(p.kind(), RunKind::Developer);
    let p = Policy::from_lookup(&env_of(&[("PVEC_DEV_RUN", "1"), ("CI", "")]));
    assert_eq!(p.kind(), RunKind::Developer);
}

#[test]
fn issue_95_a_start_failure_is_never_skipped_not_even_in_a_developer_run() {
    // Docker is there and answered; the image or the mount is wrong. That is a bug, not a missing tool.
    let p = Policy::from_lookup(&env_of(&[("PVEC_DEV_RUN", "1")]));
    let fake = Fake::new(0, 1, 0);
    let err = Container::start(&fake.config()).unwrap_err();
    assert!(matches!(p.decide(err), Decision::Fail(ContainerError::StartFailed(_))));
    let err = ContainerError::UnpinnedImage("alpine".into());
    assert!(matches!(p.decide(err), Decision::Fail(_)));
}
