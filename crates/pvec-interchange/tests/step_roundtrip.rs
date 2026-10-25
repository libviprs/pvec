//! Independent STEP round-trip validation (issue P6.3, pvec#20): the exported files are read by OpenCASCADE
//! (OCCT 7.6 through the FreeCAD Python module of Debian bookworm's `freecad-python3`) in a container, and what
//! it finds is compared with the PVec curves that were exported, structurally and geometrically.
//!
//! The tests that need no container run everywhere, against evaluator reports built in the test. The one that
//! drives the real evaluator needs a docker daemon, so it is `#[ignore]` and runs in the `docker-tests` stage of
//! `gate-fleet.sh local-ci pvec` on mars (x64) and mercury (arm64), like `step_19_reader.rs`.

use pvec_entity::EntityId;
use pvec_geometry::{Curve, Curve2d, Point2, Transform2};
use pvec_interchange::step::verify::*;
use pvec_interchange::step::{StepEntity, StepOptions, export};
use pvec_interchange::{ErrorBound, Fidelity};
use std::fmt::Write as _;

mod common;
use common::*;

/// What an evaluator would print for `curve` if it agreed with PVec exactly, built from the PVec side with the
/// edge's own domain, so a test can then corrupt one field of it.
fn mirror(curve: &Curve) -> Vec<String> {
    let d = curve.domain();
    let f = |x: f64| format!("{x:?}");
    let pt = |p: Point2| format!("{},{}", f(p.x), f(p.y));
    let mut head = format!(
        "first={} last={} start={} end={} z=0.0,0.0",
        f(d.lo()),
        f(d.hi()),
        pt(curve.eval(d.lo()).unwrap()),
        pt(curve.eval(d.hi()).unwrap())
    );
    let kind = match curve {
        Curve::Circle(c) => {
            let _ = write!(
                head,
                " length=0.0 center={} radius={} xdir={}",
                pt(c.placement().origin()),
                f(c.radius()),
                pt(Point2::new(c.placement().x_axis().x, c.placement().x_axis().y))
            );
            "Circle"
        }
        Curve::BSpline(b) => {
            let mut mults = Vec::new();
            let mut values = Vec::new();
            for k in b.knots() {
                if values.last() == Some(&f(*k)) {
                    *mults.last_mut().unwrap() += 1;
                } else {
                    values.push(f(*k));
                    mults.push(1);
                }
            }
            let poles: Vec<String> = b.control_points().iter().map(|p| pt(*p)).collect();
            let w: Vec<String> = b.control_points().iter().map(|_| "1.0".to_string()).collect();
            let _ = write!(
                head,
                " length=0.0 degree={} poles={} knots={} mults={} weights={} periodic=0.0",
                b.degree(),
                poles.join(","),
                values.join(","),
                mults.iter().map(|m| format!("{m}.0")).collect::<Vec<_>>().join(","),
                w.join(",")
            );
            "BSplineCurve"
        }
        _ => panic!("the mirror knows circles and splines"),
    };
    vec![format!("EDGE 0 {kind} {head}")]
}

fn with_samples(curve: &Curve, edge: Vec<String>, requests: &str, bend: f64) -> String {
    // the requests are `{"0":[u,...]}`; answer each from the PVec side, optionally bent by `bend` in y
    let us: Vec<f64> = requests
        .trim_matches(|c| c == '{' || c == '}')
        .split_once(':')
        .map(|(_, v)| {
            v.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.trim().parse().unwrap())
                .collect()
        })
        .unwrap_or_default();
    let mut s = String::from("EVALUATOR freecad=0.20.2 occt=7.6.3 edges=1\n");
    for e in edge {
        s.push_str(&e);
        s.push('\n');
    }
    for u in us {
        let p = curve.eval(u).unwrap();
        let side =
            if u >= curve.domain().hi() { pvec_geometry::Side::Left } else { pvec_geometry::Side::Right };
        let t = curve.derivative(u, 1, side).unwrap();
        let n = (t.x * t.x + t.y * t.y).sqrt();
        let _ = writeln!(s, "SAMPLE 0 {u:?} {:?},{:?},{:?},{:?}", p.x, p.y + bend, t.x / n, t.y / n);
    }
    s
}

fn judge(id: u32, curve: &Curve, fidelity: Fidelity, edge: Vec<String>, bend: f64) -> Verdict {
    let subject = [Subject { id: eid(id), curve, fidelity }];
    let first = Report::parse(&format!("EVALUATOR freecad=0.20.2 occt=7.6.3 edges=1\n{}\n", edge.join("\n")))
        .unwrap();
    let req = requests(&subject, &first).unwrap();
    let report = Report::parse(&with_samples(curve, edge, &req, bend)).unwrap();
    compare(&subject, &report).unwrap()
}

#[test]
fn a_report_that_agrees_with_pvec_is_a_pass_in_both_classes() {
    let c = spline();
    let v = judge(BSPLINE, &c, Fidelity::Exact, mirror(&c), 0.0);
    assert!(v.passed(), "{v:#?}");
    assert_eq!(v.exact_passes, 1);
    assert!(v.structural_checked > 0 && v.geometric_checked > 0, "{v:#?}");
}

#[test]
fn a_flipped_knot_multiplicity_is_a_structural_disagreement_with_all_six_fields() {
    let c = spline();
    let edge: Vec<String> =
        mirror(&c).iter().map(|l| l.replace("mults=4.0,1.0,1.0,4.0", "mults=4.0,1.0,2.0,3.0")).collect();
    assert_ne!(edge, mirror(&c), "the corruption must change the report");
    let v = judge(BSPLINE, &c, Fidelity::Exact, edge, 0.0);
    assert!(!v.passed());
    assert_eq!(v.exact_passes, 0);
    let d = v.structural.first().expect("a structural disagreement");
    assert_eq!(d.class, Class::Structural);
    assert_eq!(d.entity, eid(BSPLINE));
    assert!(d.pvec.contains("mult") && d.step.contains("mult"), "{d:?}");
    assert!(!d.sample.is_empty());
    assert!(d.tolerance > 0.0);
}

#[test]
fn a_dropped_weight_is_a_structural_disagreement() {
    let c = rational(vec![1.0, 0.5, 2.0, 1.0, 1.5]);
    let Curve::RationalBSpline(r) = &c else { unreachable!() };
    let edge = mirror(&Curve::BSpline(
        pvec_geometry::BSpline::new(r.degree(), r.control_points().to_vec(), r.knots().to_vec()).unwrap(),
    ));
    // the mirror writes unit weights where the rational curve has 0.5, 2.0 and 1.5: the weights are dropped
    let v = judge(RATIONAL, &c, Fidelity::Exact, edge, 0.0);
    assert!(v.structural.iter().any(|d| d.pvec.contains("weight")), "{v:#?}");
}

#[test]
fn a_deviating_sample_is_a_geometric_disagreement_and_not_a_structural_one() {
    let c = spline();
    let v = judge(BSPLINE, &c, Fidelity::Exact, mirror(&c), 0.5);
    assert!(v.structural.is_empty(), "{v:#?}");
    let d = v.geometric.first().expect("a geometric disagreement");
    assert_eq!(d.class, Class::Geometric);
    assert!(d.sample.contains("u="), "{d:?}");
    assert!(!v.passed());
}

#[test]
fn a_bounded_approximation_is_never_an_exactness_pass() {
    let c = circle(0.0, 0.0, 5.0);
    let bound = ErrorBound::new(0.25).unwrap();
    // within its bound: a display pass, and still not an exact pass
    let ok = judge(CIRCLE, &c, Fidelity::BoundedApproximation(bound), mirror(&c), 0.125);
    assert!(ok.passed(), "{ok:#?}");
    assert_eq!(ok.exact_passes, 0, "an approximated entity must not count as an exact round trip");
    assert!(ok.display_checked > 0 && ok.geometric_checked == 0, "{ok:#?}");
    // beyond its bound: a display disagreement carrying the bound
    let bad = judge(CIRCLE, &c, Fidelity::BoundedApproximation(bound), mirror(&c), 0.5);
    let d = bad.display.first().expect("a display disagreement");
    assert_eq!(d.class, Class::Display);
    assert_eq!(d.tolerance, 0.25);
}

#[test]
fn a_report_without_the_evaluator_line_or_with_the_wrong_edge_count_is_refused() {
    assert!(Report::parse("EDGE 0 Circle first=0.0\n").is_err());
    let c = circle(0.0, 0.0, 5.0);
    let subject = [Subject { id: eid(CIRCLE), curve: &c, fidelity: Fidelity::Exact }];
    let none = Report::parse("EVALUATOR freecad=0.20.2 occt=7.6.3 edges=0\n").unwrap();
    assert!(compare(&subject, &none).is_err(), "an entity with no edge is a merged or dropped entity");
}

#[test]
fn the_comparison_declares_no_tolerance_of_its_own() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/step/verify.rs")).unwrap();
    let code: String =
        src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    assert!(code.contains("Policy::V1"), "the comparison must take its tolerances from the numeric policy");
    assert!(!code.contains("EPSILON"));
    for (n, line) in code.lines().enumerate() {
        let l = line.trim_start();
        if l.starts_with("const ") || l.starts_with("pub const ") || l.starts_with("static ") {
            assert!(!l.contains("f64"), "line {n}: a float constant is a local tolerance: {l}");
        }
        // an exponent-form literal such as 1e-9 is a tolerance
        let b = line.as_bytes();
        for i in 1..b.len().saturating_sub(1) {
            if (b[i] == b'e' || b[i] == b'E')
                && b[i - 1].is_ascii_digit()
                && (b[i + 1] == b'-' || b[i + 1].is_ascii_digit())
            {
                let start = line[..i]
                    .rfind(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_'))
                    .map_or(0, |p| p + 1);
                let ident_before = start > 0 && (b[start - 1].is_ascii_alphabetic() || b[start - 1] == b'_');
                assert!(ident_before, "line {n}: an exponent literal is a tolerance: {line}");
            }
        }
    }
}

// ---- the real evaluator ---------------------------------------------------------------------------------

/// Overrides the image, for trying a rebuild before its digest is pinned below. The hermetic fixture still
/// insists on a digest.
const IMAGE_ENV: &str = "PVEC_STEP_EVAL_IMAGE";

/// The versions the suite is recorded against, as the container reports them.
const EVALUATOR: &str = "freecad=0.20.2 occt=7.6.3";

fn image() -> String {
    std::env::var(IMAGE_ENV).unwrap_or_else(|_| EVAL_IMAGE.to_string())
}

/// The org image built from `tools/step-eval/Dockerfile` (`tools/step-eval/publish.sh`), pinned by the digest of its
/// multi-platform index (amd64 and arm64).
const EVAL_IMAGE: &str =
    "ghcr.io/libviprs/pvec-step-eval@sha256:957058fc03489500ae9866b795ba4ece8dc43305042c916731908d256c07c0dc";

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn occt_reads_every_fixture_back_and_a_corrupted_mapping_turns_it_red() {
    use pvec_hermetic::{Config, Container, Decision, Policy as RunPolicy};
    use std::fs;
    let work = std::env::temp_dir().join(format!("pvec-step-20-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(work.join("out")).unwrap();
    fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/step-eval/evaluate.py"),
        work.join("evaluate.py"),
    )
    .unwrap();

    let similarity =
        Transform2::rotate(0.5).unwrap().then(&Transform2::translate(10.0, -4.0).unwrap()).unwrap();
    let affine = Transform2::new([1.5, 0.5, -0.25, 2.0, 40.0, -10.0]).unwrap();
    let mut cases: Vec<(String, Vec<(EntityId, Curve)>)> = Vec::new();
    for (name, t) in [("identity", Transform2::IDENTITY), ("similarity", similarity), ("affine", affine)] {
        let ents: Vec<StepEntity> =
            all_entities().into_iter().map(|(i, c)| StepEntity::exact(i, c)).collect();
        let opts = StepOptions::new("eval.stp", "2026-10-24T12:00:00").unwrap().with_transform(t);
        let x = export(&ents, &opts).unwrap();
        fs::write(work.join(format!("out/{name}.stp")), &x.part21).unwrap();
        let moved =
            all_entities().into_iter().filter_map(|(i, c)| Some((i, c.transform(&t).ok()?.0))).collect();
        cases.push((name.to_string(), moved));
    }
    // the corrupted mappings: a multiplicity flipped on the spline, a weight dropped on the rational curve
    let clean = fs::read_to_string(work.join("out/identity.stp")).unwrap();
    let flipped = clean.replacen("(4,1,1,4),(0.,0.25,0.75,1.0)", "(4,1,2,3),(0.,0.25,0.75,1.0)", 1);
    let dropped = clean.replacen(
        "RATIONAL_B_SPLINE_CURVE((1.0,0.5,2.0,1.0,1.5))",
        "RATIONAL_B_SPLINE_CURVE((1.0,1.0,2.0,1.0,1.5))",
        1,
    );
    assert_ne!(flipped, clean);
    assert_ne!(dropped, clean);
    fs::write(work.join("out/flipped.stp"), flipped).unwrap();
    fs::write(work.join("out/dropped.stp"), dropped).unwrap();
    let identity = cases[0].1.clone();

    let cfg = Config { image: image(), mount: work.clone(), search_path: None };
    let c = match Container::start_with_tmp(&cfg, &["sleep", "900"]) {
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
    let judge_file = |file: &str, curves: &[(EntityId, Curve)]| {
        let subjects: Vec<Subject> =
            curves.iter().map(|(id, c)| Subject { id: *id, curve: c, fidelity: Fidelity::Exact }).collect();
        let structure = run(file, None);
        eprintln!("{file}: {}", structure.evaluator());
        assert!(
            structure.evaluator().contains(EVALUATOR),
            "the suite is recorded against {EVALUATOR}: {}",
            structure.evaluator()
        );
        let req = requests(&subjects, &structure).unwrap();
        let report = run(file, Some(&req));
        compare(&subjects, &report).unwrap()
    };

    for (name, curves) in &cases {
        let v = judge_file(name, curves);
        eprintln!(
            "{name}: structural {} checked, {} not comparable; geometric {} samples; {} exact",
            v.structural_checked,
            v.structural_not_comparable.len(),
            v.geometric_checked,
            v.exact_passes
        );
        assert!(v.passed(), "{name}: OCCT disagrees with PVec:\n{:#?}\n{:#?}", v.structural, v.geometric);
        assert_eq!(v.exact_passes, curves.len(), "{name}");
        assert!(v.structural_checked > 0 && v.geometric_checked > 0);
    }
    let v = judge_file("flipped", &identity);
    assert!(
        v.structural.iter().any(|d| d.entity == eid(BSPLINE))
            && v.geometric.iter().any(|d| d.entity == eid(BSPLINE)),
        "a flipped multiplicity must be red in both classes: {v:#?}"
    );
    let v = judge_file("dropped", &identity);
    assert!(
        v.structural.iter().any(|d| d.entity == eid(RATIONAL))
            && v.geometric.iter().any(|d| d.entity == eid(RATIONAL)),
        "a dropped weight must be red in both classes: {v:#?}"
    );

    let name = c.name().to_string();
    c.stop().unwrap();
    let ps = std::process::Command::new("docker")
        .args(["ps", "-a", "--filter", &format!("name={name}"), "--format", "{{.Names}}"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ps.stdout).trim().is_empty(), "the container is still there");
    let _ = fs::remove_dir_all(&work);
}
