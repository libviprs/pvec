//! GML schema validation (issue P6.4, pvec#21): every fixture export validates against the GML 3.2.1
//! schema, in a container the test starts and removes within the run.
//!
//! The test needs a docker daemon and the container image, so it is `#[ignore]` by default and a lane
//! that has docker runs it with `--include-ignored`: the `docker-tests` stage of `gate-fleet.sh local-ci pvec`
//! does that on mars (x64) and mercury (arm64), and `tools/schema-validate.sh` does it by hand on MARS, from
//! inside a toolchain container that shares the host's docker socket. With docker missing it follows the
//! `pvec-hermetic` policy: a developer run (`PVEC_DEV_RUN=1`) skips, anything else fails.

use pvec_geometry::Transform2;
use pvec_interchange::gml::export;
use std::fs;
use std::path::{Path, PathBuf};

mod common;
use common::*;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// ---- validation against the GML 3.2.1 schema, in a container ----------------------------------------------------

/// A public image pinned by digest (the multi-platform index of `eclipse-temurin:21.0.5_11-jdk-alpine`).
/// The JDK's own XML Schema validator does the work, so no package is installed at run time.
const VALIDATOR_IMAGE: &str =
    "eclipse-temurin@sha256:ee09bfd4218b1296231588981fd1e4f74843ca585d5fd6a37bf6078e34c847c7";

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored (tools/schema-validate.sh does it by hand on MARS)"]
fn every_fixture_validates_against_the_gml_schema_in_a_container_that_is_removed_afterwards() {
    use pvec_hermetic::{Config, Container, Decision, Policy};
    let work = std::env::temp_dir().join(format!("pvec-gml-21-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    copy_tree(&repo().join("tests/schemas/gml-3.2.1"), &work.join("schemas"));
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/Validate.java"),
        work.join("Validate.java"),
    )
    .unwrap();
    fs::create_dir_all(work.join("out")).unwrap();

    let t = Transform2::new([1.5, 0.5, -0.25, 2.0, 40.0, -10.0]).unwrap();
    let mut files = Vec::new();
    for (name, ents, tf) in [
        ("fixtures-identity", fixtures(), Transform2::IDENTITY),
        ("fixtures-affine", fixtures(), t),
        ("empty", Vec::new(), Transform2::IDENTITY),
    ] {
        let x = export(&ents, &crs(tf), budget(0.02)).unwrap();
        fs::write(work.join(format!("out/{name}.xml")), x.xml).unwrap();
        files.push(format!("/corpus/out/{name}.xml"));
    }
    // a document the schema must refuse, so a validator that accepts everything cannot pass
    fs::write(
        work.join("out/bad.xml"),
        "<gml:MultiCurve xmlns:gml=\"http://www.opengis.net/gml/3.2\" gml:id=\"b\"><gml:notAThing/></gml:MultiCurve>",
    )
    .unwrap();

    let cfg = Config { image: VALIDATOR_IMAGE.to_string(), mount: work.clone(), search_path: None };
    let c = match Container::start_with_command(&cfg, &["sleep", "900"]) {
        Ok(c) => c,
        Err(e) => match Policy::from_env().decide(e) {
            Decision::Skip(why) => {
                eprintln!("skipped: {why}");
                let _ = fs::remove_dir_all(&work);
                return;
            }
            Decision::Fail(e) => panic!("the validation container could not start: {e}"),
        },
    };
    let name = c.name().to_string();

    let sums = c.exec(&["sh", "-c", "cd /corpus/schemas && sha256sum -c SHA256SUMS"]).unwrap();
    assert!(sums.success, "the vendored schemas changed: {}{}", sums.stdout, sums.stderr);

    let mut args = vec![
        "java",
        "-XX:-UsePerfData",
        "/corpus/Validate.java",
        "/corpus/schemas/gml/3.2.1/gml.xsd",
        "/corpus/schemas",
    ];
    args.extend(files.iter().map(String::as_str));
    let good = c.exec(&args).unwrap();
    assert!(good.success, "schema validation failed:\n{}\n{}", good.stdout, good.stderr);
    for f in &files {
        assert!(good.stdout.contains(&format!("OK {f}")), "{f} not reported valid:\n{}", good.stdout);
    }

    let bad = c
        .exec(&[
            "java",
            "-XX:-UsePerfData",
            "/corpus/Validate.java",
            "/corpus/schemas/gml/3.2.1/gml.xsd",
            "/corpus/schemas",
            "/corpus/out/bad.xml",
        ])
        .unwrap();
    assert!(
        !bad.success && bad.stdout.contains("FAIL /corpus/out/bad.xml"),
        "the validator accepted a bad document:\n{}",
        bad.stdout
    );

    c.stop().unwrap();
    let ps = std::process::Command::new("docker")
        .args(["ps", "-a", "--filter", &format!("name={name}"), "--format", "{{.Names}}"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ps.stdout).trim().is_empty(), "the container is still there");
    let _ = fs::remove_dir_all(&work);
}
