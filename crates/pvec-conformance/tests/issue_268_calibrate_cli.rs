//! Issue #268 (#263 slice 4): `pvec calibrate` sets, replaces or removes the calibration of a file in place, by a
//! rewrite to a temporary file and a rename, and its report says what the file now carries.

mod support;

use support::json::validate;
use support::*;

#[test]
fn calibrate_sets_a_manual_scale_then_removes_it() {
    let path = file("calibrate.pvec", &good_file());
    let run = pvec(&["calibrate", "--scale", "0.25", "--format", "json", &path]);
    assert_eq!(run.code, 0, "{} {}", run.stdout, run.stderr);
    let doc = run.json();
    validate(&schema(), &doc).unwrap_or_else(|e| panic!("{e}\n{}", run.stdout));
    assert_eq!(doc.get("command").str(), "calibrate");
    assert_eq!(doc.at("result.calibration.scale.provenance").str(), "manual");
    let human = pvec(&["inspect", &path]);
    assert!(human.stdout.contains("calibration scale: 0.25 m per source unit (manual)"), "{}", human.stdout);
    assert_eq!(pvec(&["validate", &path]).code, 0);
    let run = pvec(&["calibrate", "--remove", &path]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(std::fs::read(&path).unwrap(), good_file(), "removing gives back the file as it was written");
}

#[test]
fn calibrate_states_a_plan_scale_and_a_measured_scale() {
    let path = file("calibrate-plan.pvec", &good_file());
    let run =
        pvec(&["calibrate", "--plan-scale", "1:50", "--paper-unit", "0.001", "--note", "1:50 at A1", &path]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let doc = pvec(&["inspect", "--format", "json", &path]).json();
    assert_eq!(doc.at("result.calibration.scale.provenance").str(), "plan_scale");
    assert_eq!(doc.at("result.calibration.scale.metres_per_source_unit").num(), 0.05);
    assert_eq!(doc.at("result.calibration.scale.note").str(), "1:50 at A1");
    let run = pvec(&["calibrate", "--measured", "0,0,3,4", "--distance", "10", &path]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let doc = pvec(&["inspect", "--format", "json", &path]).json();
    assert_eq!(doc.at("result.calibration.scale.provenance").str(), "measured");
    assert_eq!(doc.at("result.calibration.scale.metres_per_source_unit").num(), 2.0);
}

#[test]
fn a_calibration_the_rules_refuse_exits_64_and_leaves_the_file_alone() {
    let path = file("calibrate-bad.pvec", &good_file());
    let run = pvec(&["calibrate", "--scale", "-1", &path]);
    assert_eq!(run.code, 64, "{}", run.stdout);
    assert_eq!(pvec(&["calibrate", &path]).code, 64, "no calibration given");
    let run = pvec(&["calibrate", "--measured", "1,1,1,1", "--distance", "5", &path]);
    assert_eq!(run.code, 2, "{} {}", run.stdout, run.stderr);
    assert!(run.stdout.contains("CAL-06") || run.stderr.contains("CAL-06"), "{} {}", run.stdout, run.stderr);
    assert_eq!(std::fs::read(&path).unwrap(), good_file());
}

#[test]
fn flags_that_do_not_make_one_calibration_are_usage_errors() {
    let path = file("calibrate-flags.pvec", &good_file());
    for args in [
        &["calibrate", "--remove", "--scale", "1", &path][..],
        &["calibrate", "--scale", "1", "--plan-scale", "1:50", "--paper-unit", "0.001", &path],
        &["calibrate", "--remove", "--note", "x", &path],
        &["calibrate", "--measured", "1,2,x,3,4", "--distance", "5", &path],
        &["calibrate", "--measured", "1,2,3", "--distance", "5", &path],
    ] {
        assert_eq!(pvec(args).code, 64, "{args:?}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), good_file());
    // a zero in a plan scale's ratio is the calibration's to refuse, as CAL-10
    let run = pvec(&["calibrate", "--plan-scale", "0:100", "--paper-unit", "0.001", &path]);
    assert_eq!(run.code, 2, "{}", run.stdout);
    assert!(run.stdout.contains("CAL-10"), "{}", run.stdout);
}
