//! Issue #120: the generator's refusal of the seven instance fixtures points at the recorded decision.

mod common;
use common::*;
use pvec_testkit::generate::{Cause, generate_all};

#[test]
fn issue_120_the_refusal_of_an_instance_fixture_cites_adr_0017() {
    let outcomes = generate_all(&bundle()).unwrap();
    let mut seen = 0;
    for o in outcomes {
        if let Err(r) = o.result {
            if let Cause::MissingCapability { capability, what } = &r.cause {
                if capability == "affine_transform_2d" {
                    seen += 1;
                    assert!(what.contains("ADR 0017"), "{}: {what}", o.fixture);
                }
            }
        }
    }
    assert_eq!(seen, 7);
}
