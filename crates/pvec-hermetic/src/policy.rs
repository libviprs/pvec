//! Dev-skip versus CI-fail.
//!
//! Skipping a container-backed test because Docker is missing is a convenience for a developer's
//! own machine. It has to be asked for (`PVEC_DEV_RUN=1`), and `CI` overrides it, so a CI run that
//! cannot start its container fails instead of quietly reporting green on nothing.

use crate::container::ContainerError;

/// What kind of run this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    /// A developer run, marked with `PVEC_DEV_RUN` and not inside CI.
    Developer,
    /// Anything else, CI included. This is the default.
    Strict,
}

/// What to do about a container that could not start.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// Skip, with the reason to print.
    Skip(String),
    /// Fail the run.
    Fail(ContainerError),
}

/// The policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    kind: RunKind,
}

impl Policy {
    /// Reads the policy from the process environment.
    pub fn from_env() -> Policy {
        Policy::from_lookup(&|k| std::env::var(k).ok())
    }

    /// Reads the policy from an environment lookup. `PVEC_DEV_RUN` is `1` or `true` for a developer
    /// run; a `CI` that is set to anything but empty, `0` or `false` wins over it.
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Policy {
        let dev = matches!(get("PVEC_DEV_RUN").as_deref(), Some("1" | "true"));
        let ci = get("CI").is_some_and(|v| !matches!(v.as_str(), "" | "0" | "false"));
        Policy { kind: if dev && !ci { RunKind::Developer } else { RunKind::Strict } }
    }

    /// The run kind.
    pub fn kind(&self) -> RunKind {
        self.kind
    }

    /// Decides what a start failure means. Only a missing or unresponsive Docker can be skipped, and
    /// only in a developer run. A container that Docker refused to start is a bug in the setup.
    pub fn decide(&self, err: ContainerError) -> Decision {
        match (&err, self.kind) {
            (ContainerError::Unavailable(why), RunKind::Developer) => {
                Decision::Skip(format!("docker is unavailable ({why}); skipped because PVEC_DEV_RUN is set"))
            }
            _ => Decision::Fail(err),
        }
    }
}
