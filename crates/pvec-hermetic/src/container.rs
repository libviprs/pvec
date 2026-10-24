//! The container fixture.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

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
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ContainerError {}

/// What to start.
#[derive(Clone, Debug)]
pub struct Config {
    /// Image, pinned as `name@sha256:...`.
    pub image: String,
    /// Directory mounted read-only at `/corpus`.
    pub mount: PathBuf,
    /// Search path for the `docker` client; `None` uses the process `PATH`.
    pub search_path: Option<OsString>,
}

/// A running container. Dropping it removes it.
#[derive(Debug)]
pub struct Container {
    name: String,
}

impl Container {
    /// Starts the container.
    pub fn start(_config: &Config) -> Result<Container, ContainerError> {
        unimplemented!("red")
    }
    /// The container name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Removes it now and reports a failure.
    pub fn stop(self) -> Result<(), ContainerError> {
        unimplemented!("red")
    }
}
