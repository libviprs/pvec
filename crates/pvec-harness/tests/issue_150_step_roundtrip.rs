//! Issue #150: the STEP round trip over every authored fixture of the corpus, and its verdicts in the gates.
//!
//! The tests without a container run everywhere. The one that drives the real evaluator needs a docker daemon, so
//! it is `#[ignore]` and runs in the `docker-tests` stage of `gate-fleet.sh local-ci pvec` (mars x64, mercury arm64).

mod common;

use pvec_capabilities::{EvidenceClass as C, Registry as Capabilities};
use pvec_corpus::{Bundle, Fixture, FixtureRegistry};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Curve2d, Transform2};
use pvec_harness::gates::{Gate, GateVerdict as V, Runner};
use pvec_harness::step_roundtrip::{curves_of, transform_of};
use pvec_harness::{ConformanceError, check_report, report};
use pvec_interchange::FidelityKind;
use pvec_interchange::step::verify::{Class, Disagreement, Verdict};
use pvec_interchange::step::{StepEntity, StepOptions, export};

fn corpus() -> FixtureRegistry {
    let bundle = Bundle::vendored().unwrap();
    let profile =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ISO_PROFILE.md")).unwrap();
    FixtureRegistry::load(&bundle, &Capabilities::v1(), &profile).unwrap()
}

/// The fixtures the round trip covers: authored mathematics, and a curve a mapper is meant to emit.
fn covered(corpus: &FixtureRegistry) -> Vec<&Fixture> {
    corpus.fixtures().iter().filter(|f| f.expected.is_some() && !f.must_not_emit_curve).collect()
}

fn disagreement(class: Class) -> Disagreement {
    Disagreement {
        entity: EntityId::new(1).unwrap(),
        class,
        pvec: "mults=4,1,1,4".to_string(),
        step: "mults=4,1,2,3".to_string(),
        sample: "edge 0".to_string(),
        tolerance: 0.5,
    }
}

#[test]
fn issue_150_every_covered_fixture_becomes_one_curve_per_entity() {
    let corpus = corpus();
    let fixtures = covered(&corpus);
    assert!(fixtures.len() >= 29, "the vendored bundle has 30 authored fixtures, one of them excluded");
    for f in fixtures {
        let expected = f.expected.as_ref().unwrap();
        let curves = curves_of(expected).unwrap_or_else(|e| panic!("{}: {e}", f.id));
        assert_eq!(curves.len(), expected.entities.len(), "{}", f.id);
        let ids: Vec<u32> = curves.iter().map(|(id, _)| id.index()).collect();
        assert_eq!(ids, (1..=curves.len() as u32).collect::<Vec<_>>(), "{}: ids number from 1", f.id);
    }
}

#[test]
fn issue_150_the_curves_are_the_authored_mathematics() {
    let corpus = corpus();
    let of = |id: &str| curves_of(corpus.get(id).unwrap().expected.as_ref().unwrap()).unwrap();
    // a circle keeps its radius and centre
    let Curve::Circle(c) = &of("circle-001")[0].1 else { panic!("circle-001 is a circle") };
    assert!(c.radius() > 0.0);
    // a cubic B-spline keeps its degree and its distinct interior knots (0.35, 0.5, 0.8)
    let Curve::BSpline(b) = &of("bspline-d3-001")[0].1 else { panic!("bspline-d3-001 is a B-spline") };
    assert_eq!(b.degree(), 3);
    assert_eq!(b.control_points().len(), 7);
    assert_eq!(b.knots().len(), 11);
    // a NURBS keeps its weights
    assert!(matches!(&of("nurbs-d3-001")[0].1, Curve::RationalBSpline(_)));
    // a bulge polyline's first segment is an arc: a trimmed circle, whose chord ends on the vertices
    let bulged = &of("bulge-arc-001")[0].1;
    let d = bulged.domain();
    let (start, end) = (bulged.eval(d.lo()).unwrap(), bulged.eval(d.hi()).unwrap());
    assert!(matches!(bulged, Curve::Composite(_) | Curve::Trimmed(_)), "{bulged:?}");
    let near = |a: f64, b: f64| pvec_numeric::Policy::V1.near(a, b, 5.0);
    assert!(near(start.x, 1.0) && near(start.y, 0.5), "{start:?}");
    assert!(near(end.x, 4.5) && near(end.y, 2.25), "{end:?}");
}

#[test]
fn issue_150_the_authored_transforms_read_as_the_maps_the_record_composed() {
    let corpus = corpus();
    let t = |id: &str| transform_of(corpus.get(id).unwrap().expected.as_ref().unwrap()).unwrap();
    for f in covered(&corpus) {
        transform_of(f.expected.as_ref().unwrap()).unwrap_or_else(|e| panic!("{}: {e}", f.id));
    }
    assert_eq!(t("line-001"), Transform2::IDENTITY);
    assert_eq!(t("translation-001"), Transform2::translate(3.5, -1.25).unwrap());
    // the chain's composed matrix: [0 -1 | 4], [2 0 | 1]
    assert_eq!(t("nested-blocks-001"), Transform2::new([0.0, -1.0, 2.0, 0.0, 4.0, 1.0]).unwrap());
    // a reflection keeps its sign
    assert_eq!(t("reflection-001"), Transform2::new([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap());
}

#[test]
fn issue_150_every_covered_curve_exports_as_an_exact_step_entity() {
    let corpus = corpus();
    for f in covered(&corpus) {
        let curves = curves_of(f.expected.as_ref().unwrap()).unwrap_or_else(|e| panic!("{}: {e}", f.id));
        let entities: Vec<StepEntity> = curves.into_iter().map(|(i, c)| StepEntity::exact(i, c)).collect();
        let x = export(&entities, &StepOptions::new("corpus.stp", "2026-10-25T12:00:00").unwrap())
            .unwrap_or_else(|e| panic!("{}: {e:?}", f.id));
        for r in x.ledger.records() {
            assert!(
                matches!(r.fidelity().kind(), FidelityKind::Exact | FidelityKind::ExactReparameterized),
                "{}: an entity the corpus calls exact was written as {:?}",
                f.id,
                r.fidelity().kind()
            );
        }
    }
}

#[test]
fn issue_150_a_clean_verdict_is_a_pass_in_both_gates_and_keeps_the_fixtures_class() {
    let mut runner = Runner::new();
    let v = Verdict { structural_checked: 12, geometric_checked: 90, exact_passes: 1, ..Verdict::default() };
    runner.step_roundtrip("bspline-d3-001", C::KnownTruthExact, &v).unwrap();
    let rows = runner.verdicts("bspline-d3-001");
    assert_eq!(rows, vec![(Gate::StructuralExact, V::Pass), (Gate::GeometricExact, V::Pass)]);
    assert!(runner.rows().iter().all(|r| r.class == C::KnownTruthExact));
    assert!(runner.rows().iter().any(|r| r.numbers.iter().any(|(n, x)| n == "samples" && *x == 90.0)));
}

#[test]
fn issue_150_the_two_classes_fail_on_their_own() {
    let mut runner = Runner::new();
    let s = Verdict {
        structural: vec![disagreement(Class::Structural)],
        structural_checked: 12,
        geometric_checked: 90,
        ..Verdict::default()
    };
    runner.step_roundtrip("a", C::KnownTruthExact, &s).unwrap();
    assert_eq!(runner.verdicts("a"), vec![(Gate::StructuralExact, V::Fail), (Gate::GeometricExact, V::Pass)]);
    let g = Verdict {
        geometric: vec![disagreement(Class::Geometric)],
        structural_checked: 12,
        geometric_checked: 90,
        ..Verdict::default()
    };
    runner.step_roundtrip("b", C::KnownTruthExact, &g).unwrap();
    assert_eq!(runner.verdicts("b"), vec![(Gate::StructuralExact, V::Pass), (Gate::GeometricExact, V::Fail)]);
}

#[test]
fn issue_150_nothing_comparable_is_routed_to_the_geometric_gate_not_passed() {
    let mut runner = Runner::new();
    let v = Verdict {
        structural_not_comparable: vec![(EntityId::new(1).unwrap(), "a closed spline is periodic in OCCT")],
        geometric_checked: 90,
        ..Verdict::default()
    };
    runner.step_roundtrip("closed-periodic-001", C::KnownTruthExact, &v).unwrap();
    assert_eq!(
        runner.verdicts("closed-periodic-001"),
        vec![(Gate::StructuralExact, V::Routed), (Gate::GeometricExact, V::Pass)]
    );
}

#[test]
fn issue_150_a_verdict_that_compared_nothing_is_not_a_pass() {
    let mut runner = Runner::new();
    runner.step_roundtrip("empty", C::KnownTruthExact, &Verdict::default()).unwrap();
    let rows = runner.verdicts("empty");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|(_, v)| *v == V::Fail), "{rows:?}");
}

#[test]
fn issue_150_a_quarantined_fixture_cannot_claim_the_round_trip() {
    let mut runner = Runner::new();
    let v = Verdict { structural_checked: 1, geometric_checked: 1, ..Verdict::default() };
    let e = runner.step_roundtrip("q", C::Quarantined, &v).unwrap_err();
    assert!(matches!(e, ConformanceError::GateRefused { .. }), "{e:?}");
    assert!(runner.rows().is_empty());
}

#[test]
fn issue_150_the_round_trip_module_declares_no_tolerance_of_its_own() {
    common::assert_no_tolerance_literal("step_roundtrip.rs", false);
}

// ---- the real evaluator ---------------------------------------------------------------------------------

/// Overrides the image, for trying a rebuild before its digest is pinned below.
const IMAGE_ENV: &str = "PVEC_STEP_EVAL_IMAGE";

/// The same pinned image `pvec-interchange/tests/step_roundtrip.rs` runs (`tools/step-eval/publish.sh`).
const EVAL_IMAGE: &str =
    "ghcr.io/libviprs/pvec-step-eval@sha256:957058fc03489500ae9866b795ba4ece8dc43305042c916731908d256c07c0dc";

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn issue_150_occt_reads_every_corpus_fixture_back_and_the_gates_record_it() {
    use pvec_hermetic::{Config, Container, Decision, Policy as RunPolicy};
    use pvec_interchange::Fidelity;
    use pvec_interchange::step::verify::{Report, Subject, compare, requests};
    use std::fs;

    let corpus = corpus();
    let fixtures = covered(&corpus);
    let work = std::env::temp_dir().join(format!("pvec-step-150-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("out")).unwrap();
    fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/step-eval/evaluate.py"),
        work.join("evaluate.py"),
    )
    .unwrap();
    // each fixture is written under the world map its record composed, and compared with the curves moved by it
    let mut curves = Vec::new();
    for f in &fixtures {
        let expected = f.expected.as_ref().unwrap();
        let t = transform_of(expected).unwrap();
        let cs = curves_of(expected).unwrap();
        let ents: Vec<StepEntity> = cs.iter().map(|(i, c)| StepEntity::exact(*i, c.clone())).collect();
        let opts = StepOptions::new("corpus.stp", "2026-10-25T12:00:00").unwrap().with_transform(t);
        let x = export(&ents, &opts).unwrap();
        fs::write(work.join(format!("out/{}.stp", f.id)), &x.part21).unwrap();
        let moved = cs
            .iter()
            .map(|(i, c)| (*i, c.transform(&t).unwrap_or_else(|e| panic!("{}: {e:?}", f.id)).0))
            .collect::<Vec<_>>();
        curves.push(moved);
    }
    // the corrupted mapping: one knot multiplicity flipped on the cubic spline
    let clean = fs::read_to_string(work.join("out/bspline-d3-001.stp")).unwrap();
    let flipped = clean.replacen("(4,1,1,1,4)", "(4,1,2,3,1)", 1);
    assert_ne!(flipped, clean, "the corruption must change the file");
    fs::write(work.join("out/flipped.stp"), flipped).unwrap();

    let image = std::env::var(IMAGE_ENV).unwrap_or_else(|_| EVAL_IMAGE.to_string());
    let cfg = Config { image, mount: work.clone(), search_path: None };
    let c = match Container::start_with_tmp(&cfg, &["sleep", "1800"]) {
        Ok(c) => c,
        Err(e) => match RunPolicy::from_env().decide(e) {
            Decision::Skip(why) => {
                eprintln!("skipped: {why}");
                let _ = fs::remove_dir_all(&work);
                return;
            }
            Decision::Fail(e) => panic!("the evaluator container could not start: {e}"),
        },
    };
    let run = |file: &str, req: Option<&str>| {
        let mut cmd = format!("python3 -I /corpus/evaluate.py /corpus/out/{file}.stp");
        if let Some(r) = req {
            cmd.push_str(&format!(" '{r}'"));
        }
        let o = c.exec(&["sh", "-c", &cmd]).unwrap();
        assert!(o.success, "{file}: the evaluator failed:\n{}\n{}", o.stdout, o.stderr);
        Report::parse(&o.stdout).unwrap()
    };
    let judge = |file: &str, cs: &[(EntityId, Curve)]| {
        let subjects: Vec<Subject> =
            cs.iter().map(|(id, c)| Subject { id: *id, curve: c, fidelity: Fidelity::Exact }).collect();
        let structure = run(file, None);
        let req = requests(&subjects, &structure).unwrap();
        compare(&subjects, &run(file, Some(&req))).unwrap()
    };

    let mut runner = Runner::new();
    for (f, cs) in fixtures.iter().zip(&curves) {
        let v = judge(&f.id, cs);
        eprintln!(
            "{}: structural {} checked, {} not comparable; geometric {} samples",
            f.id,
            v.structural_checked,
            v.structural_not_comparable.len(),
            v.geometric_checked
        );
        assert!(v.passed(), "{}: OCCT disagrees with PVec:\n{:#?}\n{:#?}", f.id, v.structural, v.geometric);
        runner.step_roundtrip(&f.id, f.class, &v).unwrap();
    }
    let bspline = fixtures.iter().position(|f| f.id == "bspline-d3-001").unwrap();
    let bad = judge("flipped", &curves[bspline]);
    assert!(!bad.structural.is_empty() && !bad.geometric.is_empty(), "a flipped multiplicity turns both red");
    let mut corrupted = Runner::new();
    corrupted.step_roundtrip("bspline-d3-001", C::KnownTruthExact, &bad).unwrap();
    assert!(corrupted.verdicts("bspline-d3-001").iter().all(|(_, v)| *v == V::Fail));

    let name = c.name().to_string();
    c.stop().unwrap();
    let ps = std::process::Command::new("docker")
        .args(["ps", "-a", "--filter", &format!("name={name}"), "--format", "{{.Names}}"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ps.stdout).trim().is_empty(), "the container is still there");
    let _ = fs::remove_dir_all(&work);

    for f in &fixtures {
        let rows = runner.verdicts(&f.id);
        assert_eq!(rows.len(), 2, "{}: one row per gate", f.id);
        assert!(rows.iter().all(|(_, v)| *v != V::Fail), "{}: {rows:?}", f.id);
    }
    let text = report::write(&runner);
    check_report(&text).unwrap();
    assert_eq!(report::read(&text).unwrap().len(), 2 * fixtures.len());
}
