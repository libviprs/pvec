//! The container fixture: start one for the run, remove it afterwards.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Why a container did not start or did not stop.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContainerError {
    /// No usable `docker`: not on the search path, or its daemon does not answer.
    Unavailable(String),
    /// Docker answered but the container did not start.
    StartFailed(String),
    /// The image is not pinned by digest.
    UnpinnedImage(String),
    /// Removal failed.
    TeardownFailed(String),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::Unavailable(w) => write!(f, "docker is unavailable: {w}"),
            ContainerError::StartFailed(w) => write!(f, "the container did not start: {w}"),
            ContainerError::UnpinnedImage(i) => {
                write!(f, "image `{i}` is not pinned by digest (name@sha256:<64 hex digits>)")
            }
            ContainerError::TeardownFailed(w) => write!(f, "the container was not removed: {w}"),
        }
    }
}

impl std::error::Error for ContainerError {}

/// What to start.
#[derive(Clone, Debug)]
pub struct Config {
    /// Image, pinned as `name@sha256:<64 hex digits>`.
    pub image: String,
    /// Directory mounted read-only at `/corpus`.
    pub mount: PathBuf,
    /// Search path for the `docker` client; `None` uses the process `PATH`.
    pub search_path: Option<OsString>,
}

/// A running container. Dropping it, on a pass, an early return or a panic, removes it.
#[derive(Debug)]
pub struct Container {
    name: String,
    docker: PathBuf,
    path: OsString,
    armed: bool,
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// The only variables the docker client sees: where to find things and which daemon to talk to.
/// No token, no registry credential, no agent socket is forwarded.
const FORWARDED: [&str; 3] = ["DOCKER_HOST", "DOCKER_CONFIG", "DOCKER_CONTEXT"];

fn pinned(image: &str) -> bool {
    image.rsplit_once("@sha256:").is_some_and(|(name, digest)| {
        !name.is_empty() && digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

fn find(search: &OsString) -> Option<PathBuf> {
    std::env::split_paths(search).map(|d| d.join("docker")).find(|p| p.is_file())
}

fn docker(bin: &Path, path: &OsString, args: &[&str]) -> std::io::Result<Output> {
    let mut attempts = 0;
    loop {
        let mut cmd = Command::new(bin);
        cmd.args(args).env_clear().env("PATH", path);
        for key in FORWARDED {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
        match cmd.stdin(Stdio::null()).output() {
            // ETXTBSY: a binary that another thread has only just finished writing is briefly
            // "text file busy" to exec. A fresh fake in a parallel test run hits it; a retry is the fix.
            Err(e) if e.raw_os_error() == Some(26) && attempts < 50 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

fn text(out: &Output) -> String {
    let s = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr });
    format!("{} ({})", s.trim(), out.status)
}

fn platform() -> &'static str {
    if cfg!(target_arch = "aarch64") { "linux/arm64" } else { "linux/amd64" }
}

impl Container {
    /// Starts the container: no network, a read-only root, the corpus mounted read-only, no
    /// environment and no credential. Nothing else of the host is visible to it.
    pub fn start(config: &Config) -> Result<Container, ContainerError> {
        if !pinned(&config.image) {
            return Err(ContainerError::UnpinnedImage(config.image.clone()));
        }
        let path = config.search_path.clone().or_else(|| std::env::var_os("PATH")).unwrap_or_default();
        let bin = find(&path)
            .ok_or_else(|| ContainerError::Unavailable("no `docker` on the search path".into()))?;
        match docker(&bin, &path, &["version"]) {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return Err(ContainerError::Unavailable(format!("`docker version` failed: {}", text(&o))));
            }
            Err(e) => return Err(ContainerError::Unavailable(format!("cannot run docker: {e}"))),
        }
        let mount = config
            .mount
            .canonicalize()
            .map_err(|e| ContainerError::StartFailed(format!("mount {}: {e}", config.mount.display())))?;
        let name = format!("pvec-hermetic-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
        let volume = format!("{}:/corpus:ro", mount.display());
        let mut c = Container { name: name.clone(), docker: bin, path, armed: true };
        let args = [
            "run",
            "-d",
            "--rm",
            "--platform",
            platform(),
            "--name",
            &name,
            "--network",
            "none",
            "--read-only",
            "-v",
            &volume,
            &config.image,
        ];
        let failure = match docker(&c.docker, &c.path, &args) {
            Ok(o) if o.status.success() => return Ok(c),
            Ok(o) => ContainerError::StartFailed(text(&o)),
            Err(e) => ContainerError::StartFailed(e.to_string()),
        };
        // Docker may have created it before failing; clean it up by name, once.
        let _ = c.remove();
        c.armed = false;
        Err(failure)
    }

    /// The container name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Removes it now and reports a failure, which a drop can only print.
    pub fn stop(mut self) -> Result<(), ContainerError> {
        self.armed = false;
        self.remove()
    }

    fn remove(&self) -> Result<(), ContainerError> {
        match docker(&self.docker, &self.path, &["rm", "-f", &self.name]) {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(ContainerError::TeardownFailed(text(&o))),
            Err(e) => Err(ContainerError::TeardownFailed(e.to_string())),
        }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        if self.armed {
            if let Err(e) = self.remove() {
                eprintln!("pvec-hermetic: {e}: container {} may be left behind", self.name);
            }
        }
    }
}
