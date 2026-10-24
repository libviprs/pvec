//! Issue P2.4: canonical `.pvec` artifacts generated from the known-truth records, through the real
//! deterministic writer. Each test is named for the acceptance criterion it holds.

mod common;

use common::*;
use pvec_container::writer::{ContainerWriter, EntityMeta, SortKey, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_corpus::expected::{Entity, EntityClass, Node};
use pvec_corpus::sha256::hex_digest;
use pvec_entity::EntityId;
use pvec_testkit::generate::{
    Artifact, Cause, GENERATOR_VERSION, Outcome, PRODUCER_PREFIX, Provenance, generate, generate_all,
    generate_record, plan, provenance,
};
use pvec_testkit::reference::{ReadEntity, read};
use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::fs;
use std::path::PathBuf;

const EMITTED: [&str; 16] = [
    "arc-seam-001",
    "bezier-d3-001",
    "bspline-d3-001",
    "bspline-d5-001",
    "circle-001",
    "closed-by-endpoint-001",
    "large-coords-001",
    "line-001",
    "lwpolyline-closed-001",
    "nonunit-weight-001",
    "nurbs-d2-offconic-001",
    "nurbs-d3-001",
    "polyline-001",
    "polyline2d-001",
    "repeated-knot-c0-001",
    "tiny-radius-001",
];

/// The fixtures the current capability set cannot hold exactly, and the capability each one is
/// missing. A refusal is the right answer for all of them: none may be approximated.
const MISSING: [(&str, &str); 13] = [
    ("bulge-arc-001", "composite_curve"),
    ("bulge-arc-neg-001", "composite_curve"),
    ("closed-periodic-001", "b_spline_curve"),
    ("ellipse-001", "ellipse"),
    ("elliptical-arc-001", "ellipse"),
    ("layer-state-001", "layer"),
    ("nested-blocks-001", "affine_transform_2d"),
    ("nonuniform-scale-circle-001", "affine_transform_2d"),
    ("nonuniform-scale-rotated-001", "affine_transform_2d"),
    ("reflection-001", "affine_transform_2d"),
    ("rotation-irrational-001", "affine_transform_2d"),
    ("translation-001", "affine_transform_2d"),
    ("uniform-scale-001", "affine_transform_2d"),
];

fn outcomes() -> Vec<Outcome> {
    generate_all(&bundle())
}

fn artifacts() -> BTreeMap<String, Artifact> {
    outcomes().into_iter().filter_map(|o| o.result.ok().map(|a| (o.fixture, a))).collect()
}

#[test]
fn every_fixture_of_the_bundle_is_accounted_for_with_its_documented_outcome() {
    let all = outcomes();
    assert_eq!(all.len(), 33, "30 registered fixtures and 3 quarantined entries");
    let mut emitted: Vec<&str> = Vec::new();
    for o in &all {
        match &o.result {
            Ok(_) => emitted.push(&o.fixture),
            Err(r) => {
                assert_eq!(r.fixture, o.fixture);
                let text = r.to_string();
                assert!(text.contains(&o.fixture), "the refusal names the fixture: {text}");
                if let Some((_, cap)) = MISSING.iter().find(|(f, _)| *f == o.fixture) {
                    match &r.cause {
                        Cause::MissingCapability { capability, .. } => {
                            assert_eq!(capability, cap, "{}", o.fixture);
                            assert!(text.contains(cap), "the refusal names the capability: {text}");
                        }
                        other => panic!("{}: expected a missing capability, got {other:?}", o.fixture),
                    }
                } else if o.fixture == "near-singular-001" {
                    assert_eq!(r.cause, Cause::NotEmitted);
                } else if ["general-affine-001", "shear-001", "singular-001"].contains(&o.fixture.as_str()) {
                    assert_eq!(r.cause, Cause::Quarantined, "{}", o.fixture);
                } else {
                    panic!("{} was refused and should not have been: {text}", o.fixture);
                }
            }
        }
    }
    emitted.sort_unstable();
    assert_eq!(emitted, EMITTED);
}

#[test]
fn generating_twice_from_the_same_revision_is_byte_identical() {
    let (a, b) = (artifacts(), artifacts());
    assert_eq!(a.len(), EMITTED.len());
    for (id, x) in &a {
        assert!(!x.bytes.is_empty(), "{id}");
        assert_eq!(x.bytes, b[id].bytes, "{id} differs between two runs");
    }
}

#[test]
fn the_order_fixtures_are_generated_in_does_not_change_a_byte() {
    let forward = artifacts();
    let bundle = bundle();
    let registry = pvec_corpus::FixtureRegistry::load(
        &bundle,
        &pvec_capabilities::Registry::v1(),
        include_str!("../../../docs/ISO_PROFILE.md"),
    )
    .unwrap();
    let mut reversed: Vec<&pvec_corpus::Fixture> = registry.fixtures().iter().collect();
    reversed.reverse();
    let mut seen = 0;
    for f in reversed {
        if let Ok(a) = generate(f, REVISION) {
            assert_eq!(a.bytes, forward[&f.id].bytes, "{}", f.id);
            seen += 1;
        }
    }
    assert_eq!(seen, EMITTED.len());
}

/// A drawing with several entities of one hot kind in one block, so that the order they arrive in
/// would show in the bytes if the writer did not canonicalise it.
fn drawing() -> pvec_corpus::ExpectedMath {
    let mut es = Vec::new();
    for i in 0..5 {
        let y = f64::from(i) * 1.5;
        es.push(line(&format!("l{i}"), [0.0, y, 0.0], [3.0 + f64::from(i), y + 0.5, 0.0]));
    }
    es.push(circle("c0", [4.0, 4.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 1.25));
    es.push(arc("a0", [1.0, 1.0, 0.0], 2.5, 0.5, 1.5));
    es.push(polyline("p0", &[[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [3.0, 1.0, 0.0]], &[0.0, 0.0, 0.0], false));
    es.push(bspline(
        "s0",
        2,
        &[[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [2.0, -1.0, 0.0], [3.0, 1.0, 0.0]],
        &[(0.0, 3), (0.5, 1), (1.0, 3)],
        None,
        OPEN,
    ));
    es.push(bspline(
        "s1",
        2,
        &[[0.0, 0.0, 0.0], [1.0, 2.0, 0.0], [2.0, -1.0, 0.0], [3.0, 1.0, 0.0]],
        &[(0.0, 3), (0.5, 1), (1.0, 3)],
        Some(&[1.0, 0.5, 2.0, 1.0]),
        OPEN,
    ));
    record("drawing", &es)
}

#[test]
fn pushing_the_same_entities_in_any_order_gives_the_same_bytes() {
    let p = plan(&drawing(), REVISION).expect("the drawing is representable");
    assert_eq!(p.len(), 10);
    let canonical = p.write_in_order(&(0..p.len()).collect::<Vec<_>>()).unwrap();
    for order in permutations(p.len()) {
        assert_eq!(p.write_in_order(&order).unwrap(), canonical, "order {order:?}");
    }
}

#[test]
fn a_collection_ordering_left_to_arrival_turns_the_determinism_check_red() {
    // The same check with the writer's entity sort switched off: it has to notice, or it would not
    // notice a generator that forgot to sort.
    let p = plan(&drawing(), REVISION).unwrap();
    let forward: Vec<usize> = (0..p.len()).collect();
    let canonical = p.write_in_order(&forward).unwrap();
    let mut reversed = forward.clone();
    reversed.reverse();
    assert_ne!(p.write_skipping(&reversed, SortKey::EntityId).unwrap(), canonical);
}

#[test]
fn artifacts_match_the_committed_digests_on_every_architecture() {
    let digests: Vec<String> = artifacts().iter().map(|(id, a)| format!("{id} {}\n", a.sha256())).collect();
    let text = digests.concat();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/artifact-digests.txt");
    if std::env::var_os("PVEC_BLESS_ARTIFACTS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &text).unwrap();
    }
    let committed = fs::read_to_string(&path)
        .expect("tests/golden/artifact-digests.txt is committed (PVEC_BLESS_ARTIFACTS=1 writes it)");
    assert_eq!(text.lines().count(), EMITTED.len());
    assert_eq!(committed, text, "a generated artifact changed; bless on purpose if that is intended");
}

fn num(n: &Node) -> f64 {
    n.as_scalar().expect("a scalar").value()
}

fn xy(n: &Node) -> [f64; 2] {
    let l = n.as_list().expect("a point");
    [num(&l[0]), num(&l[1])]
}

fn points(n: &Node) -> Vec<[f64; 2]> {
    n.as_list().expect("points").iter().map(xy).collect()
}

fn bits(v: &[f64]) -> Vec<u64> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// What the record says the file must hold, written down independently of the generator.
fn truth(e: &Entity) -> Vec<u64> {
    let g = &e.geometry;
    let int = |k: &str| match g.get(k) {
        Some(Node::Int(i)) => usize::try_from(*i).unwrap(),
        other => panic!("{k}: {other:?}"),
    };
    let cps = |g: &Node| -> (Vec<f64>, Vec<f64>) {
        let p = points(g.get("control_points").unwrap());
        (p.iter().map(|q| q[0]).collect(), p.iter().map(|q| q[1]).collect())
    };
    match e.class {
        EntityClass::Line => {
            let (a, b) = (xy(g.get("start").unwrap()), xy(g.get("end").unwrap()));
            bits(&[a[0], a[1], b[0], b[1]])
        }
        EntityClass::Polyline => {
            let mut v = points(g.get("vertices").unwrap());
            if e.node.get("closure").and_then(|c| c.get("closed_by_endpoint")) == Some(&Node::Bool(true)) {
                v.push(v[0]);
            }
            bits(&v.iter().map(|p| p[0]).chain(v.iter().map(|p| p[1])).collect::<Vec<_>>())
        }
        EntityClass::Circle => {
            let c = xy(g.get("center").unwrap());
            bits(&[c[0], c[1], num(g.get("radius").unwrap()), 0.0, TAU])
        }
        EntityClass::CircularArc => {
            let c = xy(g.get("center").unwrap());
            bits(&[
                c[0],
                c[1],
                num(g.get("radius").unwrap()),
                num(g.get("start_angle").unwrap()),
                num(g.get("sweep_angle").unwrap()),
            ])
        }
        EntityClass::Bezier => {
            let (x, y) = cps(g);
            let d = int("degree");
            let mut k = vec![0.0; d + 1];
            k.extend(vec![1.0; d + 1]);
            [bits(&k), bits(&x), bits(&y)].concat()
        }
        EntityClass::Bspline | EntityClass::Nurbs => {
            let (x, y) = cps(g);
            let mut k = Vec::new();
            for m in g.get("knots").unwrap().as_list().unwrap() {
                let v = num(m.get("value").unwrap());
                let Some(Node::Int(mult)) = m.get("multiplicity") else { panic!("multiplicity") };
                k.extend(std::iter::repeat_n(v, usize::try_from(*mult).unwrap()));
            }
            let w: Vec<f64> = if e.class == EntityClass::Nurbs {
                g.get("weights").unwrap().as_list().unwrap().iter().map(num).collect()
            } else {
                Vec::new()
            };
            [bits(&k), bits(&x), bits(&y), bits(&w)].concat()
        }
        other => panic!("{other:?} is not emitted"),
    }
}

#[test]
fn every_stored_real_is_the_bit_pattern_the_record_authored() {
    let bundle = bundle();
    let registry = pvec_corpus::FixtureRegistry::load(
        &bundle,
        &pvec_capabilities::Registry::v1(),
        include_str!("../../../docs/ISO_PROFILE.md"),
    )
    .unwrap();
    let all = artifacts();
    assert_eq!(all.len(), EMITTED.len(), "nothing generated means nothing was checked");
    for (id, art) in all {
        let record = registry.get(&id).unwrap().expected.as_ref().unwrap();
        let opened = read(&art.bytes).unwrap_or_else(|e| panic!("{id} does not read back: {e}"));
        assert_eq!(opened.entities.len(), record.entities.len(), "{id}");
        for (i, (got, want)) in opened.entities.iter().zip(&record.entities).enumerate() {
            assert_eq!(art.ids[i], (want.id.clone(), got.id()), "{id}: the id map");
            assert_eq!(got.bits(), truth(want), "{id}/{}: stored numbers differ from the record", want.id);
        }
    }
}

#[test]
fn a_closed_polyline_repeats_its_first_vertex_and_a_closed_spline_keeps_its_flag() {
    let all = artifacts();
    let open = read(&all["lwpolyline-closed-001"].bytes).unwrap();
    let ReadEntity::Polyline { x, y, .. } = &open.entities[0] else { panic!("a polyline") };
    assert_eq!((x.len(), y.len()), (5, 5), "four vertices and the closing one");
    assert_eq!((x[4].to_bits(), y[4].to_bits()), (x[0].to_bits(), y[0].to_bits()));
    let closed = read(&all["closed-by-endpoint-001"].bytes).unwrap();
    assert!(matches!(&closed.entities[0], ReadEntity::Spline { closed: true, .. }));
    let plain = read(&all["bspline-d3-001"].bytes).unwrap();
    assert!(matches!(&plain.entities[0], ReadEntity::Spline { closed: false, .. }));
}

#[test]
fn every_artifact_records_fixture_id_corpus_revision_and_generator_version() {
    let all = artifacts();
    assert_eq!(all.len(), EMITTED.len(), "nothing generated means nothing was checked");
    for (id, art) in all {
        let p = provenance(&art.bytes).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert_eq!(
            p,
            Provenance {
                fixture_id: id.clone(),
                corpus_revision: "r0-open".to_string(),
                generator_version: GENERATOR_VERSION.to_string()
            }
        );
        assert_eq!(art.corpus_revision, "r0-open");
        assert_eq!(art.fixture_id, id);
    }
}

fn written_with(producer_id: &str, producer_version: &str, revision: Option<&str>) -> Vec<u8> {
    let mut w = ContainerWriter::new(WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: producer_id.to_string(),
            producer_version: producer_version.to_string(),
            corpus_revision: revision.map(str::to_string),
        },
        bounds: None::<Bounds>,
        capabilities: Vec::new(),
    });
    let b = Bounds { min: [0.0, 0.0], max: [1.0, 1.0] };
    w.push_line(EntityMeta { id: EntityId::new(0).unwrap(), style: 0, cell: 0, bounds: b }, [0.0, 0.0], [1.0, 1.0])
        .unwrap();
    w.finish().unwrap()
}

#[test]
fn a_missing_provenance_field_fails_validation_and_names_the_field() {
    let good = written_with(&format!("{PRODUCER_PREFIX}line-001"), GENERATOR_VERSION, Some("r0-open"));
    assert_eq!(provenance(&good).unwrap().fixture_id, "line-001");
    for (bytes, field) in [
        (written_with(&format!("{PRODUCER_PREFIX}line-001"), GENERATOR_VERSION, None), "corpus_revision"),
        (written_with(&format!("{PRODUCER_PREFIX}line-001"), GENERATOR_VERSION, Some("")), "corpus_revision"),
        (written_with(PRODUCER_PREFIX, GENERATOR_VERSION, Some("r0-open")), "fixture_id"),
        (written_with("somebody-else", GENERATOR_VERSION, Some("r0-open")), "fixture_id"),
        (written_with(&format!("{PRODUCER_PREFIX}line-001"), "", Some("r0-open")), "generator_version"),
    ] {
        assert_eq!(provenance(&bytes).unwrap_err().field, field);
    }
    assert!(provenance(b"not an artifact").is_err());
}

fn refused(math: &pvec_corpus::ExpectedMath) -> pvec_testkit::generate::Refusal {
    generate_record(math, REVISION).expect_err("the record has to be refused, not approximated")
}

fn missing(math: &pvec_corpus::ExpectedMath, entity: &str, capability: &str) {
    let r = refused(math);
    assert_eq!(r.entity.as_deref(), Some(entity));
    match &r.cause {
        Cause::MissingCapability { capability: c, .. } => assert_eq!(c, capability),
        other => panic!("expected a missing capability, got {other:?}"),
    }
    assert!(r.to_string().contains(capability), "{r}");
    assert!(r.to_string().contains(entity), "{r}");
}

#[test]
fn geometry_the_capability_set_cannot_hold_is_refused_with_the_capability_named() {
    let o = [0.0, 0.0, 0.0];
    let z = [0.0, 0.0, 1.0];
    let x = [1.0, 0.0, 0.0];
    // off the drawing plane: there is no implicit dimension promotion
    missing(&record("r", &[line("up", [0.0, 0.0, 0.5], [1.0, 1.0, 0.5])]), "up", "line");
    missing(&record("r", &[circle("tilted", o, [0.0, 1.0, 0.0], x, 1.0)]), "tilted", "circle");
    missing(&record("r", &[circle("flipped", o, [0.0, 0.0, -1.0], x, 1.0)]), "flipped", "circle");
    // a rotated frame needs an angle whose last bit belongs to libm
    missing(&record("r", &[circle("turned", o, z, [0.0, 1.0, 0.0], 1.0)]), "turned", "circle");
    missing(&record("r", &[ellipse("slanted", o, [0.96, 0.28, 0.0], 3.0, 2.0)]), "slanted", "ellipse");
    // arcs inside a polyline are a composite of lines and trimmed circles
    let verts = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 2.0, 0.0]];
    missing(&record("r", &[polyline("bulged", &verts, &[0.0, 0.5, 0.0], false)]), "bulged", "composite_curve");
    // no wrapped periodic form
    let cps = [[3.0, 0.0, 0.0], [1.5, 2.5, 0.0], [-1.5, 2.5, 0.0], [-3.0, 0.0, 0.0], [-1.5, -2.5, 0.0], [1.5, -2.5, 0.0]];
    let knots: Vec<(f64, u32)> = [0.0, 0.1, 0.25, 0.4, 0.6, 0.8, 1.0].iter().map(|k| (*k, 1)).collect();
    missing(&record("r", &[bspline("wrapped", 3, &cps, &knots, None, PERIODIC)]), "wrapped", "b_spline_curve");
}

#[test]
fn one_entity_that_cannot_be_held_refuses_the_whole_fixture_and_no_bytes_come_out() {
    let m = record(
        "mixed",
        &[line("ok", [0.0, 0.0, 0.0], [1.0, 1.0, 0.0]), line("up", [0.0, 0.0, 1.0], [1.0, 1.0, 1.0])],
    );
    let r = refused(&m);
    assert_eq!(r.entity.as_deref(), Some("up"));
    assert!(plan(&m, REVISION).is_err());
}

#[test]
fn a_unit_other_than_the_metre_is_refused_and_so_is_a_scale_other_than_one() {
    let l = line("l", [0.0, 0.0, 0.0], [1.0, 1.0, 0.0]);
    for (m, word) in [
        (record_in_unit("u", "foot", 0.3048, std::slice::from_ref(&l)), "foot"),
        (record_in_unit("u", "metre", 2.0, &[l]), "scale"),
    ] {
        let r = refused(&m);
        match &r.cause {
            Cause::Unsupported { what } => assert!(what.contains(word), "{what}"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }
}

#[test]
fn what_the_model_refuses_comes_back_as_its_typed_error_and_is_not_repaired() {
    // two vertices closer than the degenerate length (NP-06)
    let m = record("r", &[polyline("short", &[[0.0, 0.0, 0.0], [0.0, 1e-13, 0.0]], &[0.0, 0.0], false)]);
    let r = refused(&m);
    assert_eq!(r.entity.as_deref(), Some("short"));
    assert!(matches!(r.cause, Cause::Geometry(_) | Cause::Write(_)), "{r}");
}

#[test]
fn an_axis_aligned_ellipse_is_representable_and_stored_as_authored() {
    let m = record("e", &[ellipse("e0", [2.0, 1.0, 0.0], [1.0, 0.0, 0.0], 3.0, 2.0)]);
    let a = generate_record(&m, REVISION).expect("an axis-aligned ellipse is held exactly");
    let opened = read(&a.bytes).unwrap();
    assert_eq!(opened.entities[0].bits(), bits(&[2.0, 1.0, 3.0, 2.0, 0.0, 0.0, TAU]));
}

#[test]
fn artifacts_are_reproducible_from_the_bundle_alone() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("issue-37-artifacts");
    let digest_map = |dir: &PathBuf| -> BTreeMap<String, String> {
        fs::read_dir(dir)
            .expect("the artifact directory exists")
            .map(|e| {
                let p = e.unwrap().path();
                (p.file_name().unwrap().to_string_lossy().into_owned(), hex_digest(&fs::read(&p).unwrap()))
            })
            .collect()
    };
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let first = pvec_testkit::generate::emit(&dir, &outcomes()).unwrap();
    assert_eq!(first.len(), EMITTED.len());
    let before = digest_map(&dir);
    assert_eq!(before.len(), EMITTED.len());
    // delete every artifact, rebuild from a fresh copy of the bundle, and compare
    fs::remove_dir_all(&dir).unwrap();
    fs::create_dir_all(&dir).unwrap();
    assert!(digest_map(&dir).is_empty());
    pvec_testkit::generate::emit(&dir, &generate_all(&bundle())).unwrap();
    assert_eq!(digest_map(&dir), before);
}

/// What is wrong with a dependency tree: a package that is not of this workspace, or one that reads
/// CAD or PDF, or any `libviprs` crate.
fn tree_problems(lock: &str, manifests: &[(&str, &str)]) -> Vec<String> {
    const BANNED: [&str; 14] = [
        "libviprs", "dwg", "dxf", "acadrust", "acadsharp", "pdfium", "lopdf", "pdf-", "cadex", "teigha",
        "opencascade", "occt", "oda", "step-",
    ];
    let mut out = Vec::new();
    for line in lock.lines() {
        if let Some(name) = line.strip_prefix("name = \"").and_then(|r| r.strip_suffix('"')) {
            if !name.starts_with("pvec-") {
                out.push(format!("Cargo.lock: {name} is not a crate of this workspace"));
            }
            for b in BANNED {
                if name.contains(b) {
                    out.push(format!("Cargo.lock: {name} looks like `{b}`"));
                }
            }
        }
    }
    for (path, text) in manifests {
        let mut in_deps = false;
        for line in text.lines() {
            let t = line.trim();
            if t.starts_with('[') {
                in_deps = t.ends_with("dependencies]");
            } else if in_deps && !t.starts_with('#') {
                if let Some((name, _)) = t.split_once('=') {
                    let name = name.trim();
                    for b in BANNED {
                        if name.contains(b) {
                            out.push(format!("{path}: dependency {name} looks like `{b}`"));
                        }
                    }
                }
            }
        }
    }
    out
}

#[test]
fn the_dependency_check_flags_a_libviprs_crate_and_a_cad_or_pdf_reader() {
    let lock = "[[package]]\nname = \"pvec-numeric\"\n\n[[package]]\nname = \"libviprs-core\"\n\n[[package]]\nname = \"pdfium-render\"\n";
    let manifest = "[dependencies]\nacadrust = \"1\"\npvec-numeric = { path = \"../pvec-numeric\" }\n";
    let found = tree_problems(lock, &[("crates/x/Cargo.toml", manifest)]);
    assert!(found.iter().any(|p| p.contains("libviprs-core")), "{found:?}");
    assert!(found.iter().any(|p| p.contains("pdfium-render")), "{found:?}");
    assert!(found.iter().any(|p| p.contains("acadrust")), "{found:?}");
    assert!(tree_problems("[[package]]\nname = \"pvec-numeric\"\n", &[]).is_empty());
}

#[test]
fn the_repository_tree_has_no_libviprs_crate_and_no_cad_or_pdf_reader() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let mut manifests = Vec::new();
    for e in fs::read_dir(root.join("crates")).unwrap() {
        let p = e.unwrap().path().join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&p) {
            manifests.push((p.display().to_string(), text));
        }
    }
    assert!(manifests.len() >= 9, "the scan has to see the crates");
    let refs: Vec<(&str, &str)> = manifests.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(tree_problems(&lock, &refs), Vec::<String>::new());
}
