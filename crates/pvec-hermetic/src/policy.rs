//! Dev-skip versus CI-fail.

use crate::container::ContainerError;

/// What kind of run this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    /// A developer run, marked with `PVEC_DEV_RUN`.
    Developer,
    /// Anything else, CI included.
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
    /// Reads the policy from an environment lookup.
    pub fn from_lookup(_get: &dyn Fn(&str) -> Option<String>) -> Policy {
        unimplemented!("red")
    }
    /// The run kind.
    pub fn kind(&self) -> RunKind {
        self.kind
    }
    /// Decides what a start failure means.
    pub fn decide(&self, _err: ContainerError) -> Decision {
        unimplemented!("red")
    }
}
