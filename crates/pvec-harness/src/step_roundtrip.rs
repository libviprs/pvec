//! The STEP round trip over the corpus, wired into the frozen gates (issue P6.3b, pvec#150).
//!
//! Not written yet: the red tests of `tests/issue_150_step_roundtrip.rs` come first.

use crate::error::Result;
use crate::gates::Runner;
use pvec_capabilities::EvidenceClass;
use pvec_corpus::ExpectedMath;
use pvec_entity::EntityId;
use pvec_geometry::Curve;
use pvec_interchange::step::verify::Verdict;

/// The curves a fixture's authored mathematics describes, one per entity, numbered from 1.
pub fn curves_of(_expected: &ExpectedMath) -> std::result::Result<Vec<(EntityId, Curve)>, String> {
    Err("not implemented".to_string())
}

impl Runner {
    /// Records a STEP round trip verdict under the structural and the geometric gate.
    pub fn step_roundtrip(&mut self, _fixture: &str, _class: EvidenceClass, _verdict: &Verdict) -> Result<()> {
        Ok(())
    }
}
