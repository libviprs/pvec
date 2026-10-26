//! The named seed corpus.

/// Where a seed came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// A case of the hostile corpus (P2.7).
    Hostile,
    /// A `KNOWN_TRUTH_EXACT` fixture of the vendored bundle, written by the container writer.
    KnownTruth,
    /// Container writer output of the synthetic shapes, and mutated manifests of it.
    Writer,
}

/// One seed.
#[derive(Clone, Debug)]
pub struct Seed {
    /// A unique name.
    pub name: String,
    /// Where it came from.
    pub source: Source,
    /// The bytes of an artifact.
    pub bytes: Vec<u8>,
}

/// Every seed, deterministic.
pub fn corpus() -> Vec<Seed> {
    todo!("P7.8")
}

/// The inputs for one target, taken from the seeds: the whole artifact, its header, one hot block, the framed
/// advanced blocks, a framed compressed block or the component stream set.
pub fn for_target(_target: &str, _seeds: &[Seed]) -> Vec<Vec<u8>> {
    todo!("P7.8")
}

/// What is not in the corpus yet, and why: the sources the issue names that are not on main.
pub const DEFERRED: &[(&str, &str)] = &[];
