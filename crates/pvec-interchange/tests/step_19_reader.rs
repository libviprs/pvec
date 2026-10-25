//! The independent STEP reader (issue P6.2, pvec#19): the exported files are read by a third-party
//! Part 21 implementation in a container, and the parameters it reports are compared with the curves
//! that were exported.
//!
//! The reader is `steputils` 0.1 (MIT), vendored as wheels in `tests/vendor/steputils` and run by
//! `tests/support/read_step.py` under the CPython of a pinned `python` image, with no network and a
//! read-only root. The suite records it: [`READER`] is asserted against what the container reports, and
//! the report line is printed with `--nocapture`.
//!
//! The test needs a docker daemon, so it is `#[ignore]` by default and a lane that has docker runs it with
//! `--include-ignored`: the `docker-tests` stage of `gate-fleet.sh local-ci pvec` does that on mars (x64)
//! and mercury (arm64). With docker missing it follows the `pvec-hermetic` policy: a developer run
//! (`PVEC_DEV_RUN=1`) skips, anything else fails.

use pvec_geometry::{Curve, Curve2d, Placement2, Point2, Transform2, Vector2};
use pvec_interchange::step::*;
use std::fs;
use std::path::{Path, PathBuf};

mod common;
use common::*;

/// The reader this suite is recorded against: name and version as the container reports them.
const READER: &str = "steputils 0.1";

/// A public image pinned by digest (the multi-platform index of `python:3.12-slim`, CPython 3.12.15).
const READER_IMAGE: &str = "python@sha256:a6e34c598f2467ed0e9a8d349809fcd8b5c603269512df273a0bb1784edc11b1";

const WHEELS: [&str; 3] = [
    "steputils-0.1-py3-none-any.whl",
    "pyparsing-3.3.3-py3-none-any.whl",
    "antlr4_python3_runtime-4.13.2-py3-none-any.whl",
];

fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

// ---- the canonical form, the same as read_step.py prints ------------------------------------------------

fn bits(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}

fn p(p: Point2) -> String {
    format!("p({},{})", bits(p.x), bits(p.y))
}

fn d(v: Vector2) -> String {
    format!("d({},{})", bits(v.x), bits(v.y))
}

fn a(pl: &Placement2) -> String {
    format!("a({},{})", p(pl.origin()), d(pl.x_axis()))
}

fn pts(ps: &[Point2]) -> String {
    ps.iter().map(|q| p(*q)).collect::<Vec<_>>().join(";")
}

fn flag(closed: bool) -> &'static str {
    if closed { "T" } else { "F" }
}

fn knots(k: &[f64]) -> String {
    let (mut mults, mut values) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < k.len() {
        let mut j = i;
        while j < k.len() && k[j].to_bits() == k[i].to_bits() {
            j += 1;
        }
        mults.push((j - i).to_string());
        values.push(bits(k[i]));
        i = j;
    }
    format!("{}|{}", mults.join(","), values.join(","))
}

fn canon(c: &Curve) -> String {
    match c {
        Curve::Line(l) => format!("line({},{})", p(l.origin()), d(l.direction())),
        Curve::Circle(x) => format!("circle({},{})", a(x.placement()), bits(x.radius())),
        Curve::Ellipse(e) => {
            format!("ellipse({},{},{})", a(e.placement()), bits(e.semi_axis_1()), bits(e.semi_axis_2()))
        }
        Curve::Polyline(q) => format!("poly({})", pts(q.vertices())),
        Curve::Trimmed(t) => {
            let tr = t.trim();
            let end = tr.start + tr.span;
            let (t1, t2, sense) = if tr.reversed { (end, tr.start, "F") } else { (tr.start, end, "T") };
            format!("trim({},{},{},{sense})", canon(t.basis()), bits(t1), bits(t2))
        }
        Curve::Composite(comp) => {
            format!("comp({})", comp.segments().iter().map(canon).collect::<Vec<_>>().join(";"))
        }
        Curve::Bezier(b) => {
            let dg = b.degree();
            let mut k = vec![0.0; dg + 1];
            k.extend(vec![1.0; dg + 1]);
            format!("bs({dg}|{}|{}|{})", flag(b.is_declared_closed()), pts(b.control_points()), knots(&k))
        }
        Curve::BSpline(b) => format!(
            "bs({}|{}|{}|{})",
            b.degree(),
            flag(b.is_declared_closed()),
            pts(b.control_points()),
            knots(b.knots())
        ),
        Curve::RationalBSpline(r) => format!(
            "rbs({}|{}|{}|{}|{})",
            r.degree(),
            flag(r.is_declared_closed()),
            pts(r.control_points()),
            knots(r.knots()),
            r.weights().iter().map(|w| bits(*w)).collect::<Vec<_>>().join(",")
        ),
        _ => panic!("a curve kind this test does not know"),
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

#[test]
fn the_canonical_form_of_a_curve_is_stable() {
    // not docker: the form read_step.py has to match is pinned for one curve, so a change to either side shows
    let c = circle(1.0, 2.0, 3.0);
    assert_eq!(
        canon(&c),
        "circle(a(p(3ff0000000000000,4000000000000000),d(3ff0000000000000,0000000000000000)),4008000000000000)"
    );
}

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn an_independent_part_21_reader_accepts_every_file_and_reports_the_parameters_that_were_written() {
    use pvec_hermetic::{Config, Container, Decision, Policy};
    let work = std::env::temp_dir().join(format!("pvec-step-19-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    copy_tree(&manifest().join("tests/vendor/steputils"), &work.join("wheels"));
    fs::copy(manifest().join("tests/support/read_step.py"), work.join("read_step.py")).unwrap();
    fs::create_dir_all(work.join("out")).unwrap();

    let affine = Transform2::new([1.5, 0.5, -0.25, 2.0, 40.0, -10.0]).unwrap();
    let similarity =
        Transform2::rotate(0.5).unwrap().then(&Transform2::translate(10.0, -4.0).unwrap()).unwrap();
    let mut expected: Vec<(String, Vec<String>)> = Vec::new();
    for (name, t) in [("identity", Transform2::IDENTITY), ("similarity", similarity), ("affine", affine)] {
        let ents: Vec<StepEntity> =
            all_entities().into_iter().map(|(i, c)| StepEntity::exact(i, c)).collect();
        let opts = StepOptions::new("reader.stp", "2026-10-24T12:00:00").unwrap().with_transform(t);
        let x = export(&ents, &opts).unwrap();
        fs::write(work.join(format!("out/{name}.stp")), &x.part21).unwrap();
        let lines = all_entities()
            .into_iter()
            .filter_map(|(id, c)| {
                let moved =
                    if t.matrix() == Transform2::IDENTITY.matrix() { c } else { c.transform(&t).ok()?.0 };
                Some(format!("ENTITY e{} {}", id.index(), canon(&moved)))
            })
            .collect();
        expected.push((format!("/corpus/out/{name}.stp"), lines));
    }
    let empty = export(&[], &StepOptions::new("empty.stp", "2026-10-24T12:00:00").unwrap()).unwrap();
    fs::write(work.join("out/empty.stp"), &empty.part21).unwrap();
    expected.push(("/corpus/out/empty.stp".to_string(), Vec::new()));

    // files the reader must refuse, so a reader that accepts anything cannot pass
    let good = fs::read_to_string(work.join("out/identity.stp")).unwrap();
    fs::write(work.join("out/not-step.stp"), "this is not a STEP file\n").unwrap();
    fs::write(work.join("out/truncated.stp"), &good[..good.find("ENDSEC;\nEND-ISO").unwrap()]).unwrap();
    fs::write(work.join("out/bad-ref.stp"), good.replace("#3=CARTESIAN_POINT", "#3=CARTESIAN_POINT("))
        .unwrap();

    let cfg = Config { image: READER_IMAGE.to_string(), mount: work.clone(), search_path: None };
    let c = match Container::start_with_command(&cfg, &["sleep", "900"]) {
        Ok(c) => c,
        Err(e) => match Policy::from_env().decide(e) {
            Decision::Skip(why) => {
                eprintln!("skipped: {why}");
                let _ = fs::remove_dir_all(&work);
                return;
            }
            Decision::Fail(e) => panic!("the reader container could not start: {e}"),
        },
    };
    let name = c.name().to_string();

    let sums = c.exec(&["sh", "-c", "cd /corpus/wheels && sha256sum -c SHA256SUMS"]).unwrap();
    assert!(sums.success, "the vendored wheels changed: {}{}", sums.stdout, sums.stderr);

    let pythonpath = WHEELS.iter().map(|w| format!("/corpus/wheels/{w}")).collect::<Vec<_>>().join(":");
    let run = |files: &[&str]| {
        let cmd = format!(
            "PYTHONDONTWRITEBYTECODE=1 PYTHONPATH={pythonpath} python3 /corpus/read_step.py {}",
            files.join(" ")
        );
        c.exec(&["sh", "-c", &cmd]).unwrap()
    };

    let files: Vec<&str> = expected.iter().map(|(f, _)| f.as_str()).collect();
    let good = run(&files);
    eprintln!("{}", good.stdout.lines().next().unwrap_or(""));
    assert!(good.success, "the reader failed:\n{}\n{}", good.stdout, good.stderr);
    assert!(
        good.stdout.lines().next().is_some_and(|l| l.starts_with(&format!("READER {READER},"))),
        "the suite is recorded against {READER}, the container reports:\n{}",
        good.stdout.lines().next().unwrap_or("nothing")
    );
    let mut sections = good.stdout.split("\nOK ").skip(1);
    for (file, lines) in &expected {
        let section =
            sections.next().unwrap_or_else(|| panic!("{file} was not reported OK:\n{}", good.stdout));
        let mut it = section.lines();
        let head = it.next().unwrap();
        assert!(head.starts_with(file.as_str()), "order: {head} vs {file}");
        let got: Vec<&str> = it.collect();
        assert_eq!(got.len(), lines.len(), "{file}: entity count\n{section}");
        for (g, w) in got.iter().zip(lines) {
            assert_eq!(g, w, "{file}: the reader found different parameters than were written");
        }
    }
    assert!(!good.stdout.contains("FAIL"), "{}", good.stdout);

    let bad = run(&["/corpus/out/not-step.stp", "/corpus/out/truncated.stp", "/corpus/out/bad-ref.stp"]);
    for f in ["not-step", "truncated", "bad-ref"] {
        assert!(
            bad.stdout.contains(&format!("FAIL /corpus/out/{f}.stp")),
            "the reader accepted {f}.stp:\n{}",
            bad.stdout
        );
    }
    assert!(!bad.stdout.contains("\nOK "), "{}", bad.stdout);

    c.stop().unwrap();
    let ps = std::process::Command::new("docker")
        .args(["ps", "-a", "--filter", &format!("name={name}"), "--format", "{{.Names}}"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ps.stdout).trim().is_empty(), "the container is still there");
    let _ = fs::remove_dir_all(&work);
}
