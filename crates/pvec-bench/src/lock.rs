//! The lock file and the viewport CSV, rendered from the generator so that nothing in them is typed by hand.

use crate::shape::{Shape, artifact, summarise};
use crate::{CLASSES, GENERATOR, VIEWPORTS, bounds, counts, known_truth, viewports};
use pvec_corpus::sha256::hex_digest;
use pvec_corpus::{Bundle, FixtureRegistry};
use pvec_testkit::reference::read;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The fixture commit the vendored bundle was taken from (`docs/CORPUS.md`).
pub const FIXTURE_COMMIT: &str = "1df9d1b3e75047fa3978ebd809d0128cdfb28a68";

/// One generation measurement of a `--measure` run, as a row of `measured.csv`:
/// `name,platform,host,millis,peak_rss_kib,sha256,bytes`.
struct Measured<'a> {
    platform: &'a str,
    host: &'a str,
    millis: u64,
    peak_rss_kib: u64,
    sha256: &'a str,
    bytes: u64,
}

fn parse_measured<'a>(csv: &'a str, name: &str) -> Vec<Measured<'a>> {
    csv.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f.len() == 7 && f[0] == name).then(|| Measured {
                platform: f[1],
                host: f[2],
                millis: f[3].parse().expect("millis"),
                peak_rss_kib: f[4].parse().expect("peak rss"),
                sha256: f[5],
                bytes: f[6].parse().expect("bytes"),
            })
        })
        .collect()
}

fn hex(v: f64) -> String {
    format!("\"0x{:016x}\"", v.to_bits())
}

/// What the lock says about one entry. The counts are by registry capability name.
struct Row {
    name: String,
    class: &'static str,
    entities: u64,
    obtain: &'static str,
    seed: Option<u64>,
    bytes: u64,
    sha256: String,
    counts: BTreeMap<String, u64>,
    bounds: [f64; 4],
    unit: String,
    evidence: Option<&'static str>,
    on_demand: bool,
    generation: String,
}

/// The lock file and the viewport CSV. `measured` is the text of `measured.csv`, which supplies the digest,
/// size and cost of the on-demand case, since this function never builds it.
pub fn render(measured: &str) -> Result<(String, String), String> {
    let mut rows = Vec::new();
    let (built, refused) = known_truth().map_err(|e| e.to_string())?;
    let registry = FixtureRegistry::load(
        &Bundle::vendored().map_err(|e| e.to_string())?,
        &pvec_capabilities::Registry::v1(),
        include_str!("../../../docs/ISO_PROFILE.md"),
    )
    .map_err(|e| e.to_string())?;
    for (name, bytes) in &built {
        let class = registry.get(name).ok_or(format!("{name}: not registered"))?.class.name();
        let counts = counts(bytes)?;
        rows.push(Row {
            name: name.clone(),
            class: CLASSES[0],
            entities: counts.values().sum(),
            obtain: "vendored-bundle",
            seed: None,
            bytes: bytes.len() as u64,
            sha256: hex_digest(bytes),
            counts,
            bounds: bounds(bytes)?,
            unit: format!("{:?}", read(bytes).map_err(|e| e.to_string())?.manifest.unit).to_lowercase(),
            evidence: Some(class),
            on_demand: false,
            generation: String::new(),
        });
    }
    for spec in &crate::SPECS {
        let (shape_counts, b) = summarise(&spec.shapes());
        let counts: BTreeMap<String, u64> =
            shape_counts.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let (bytes, sha256, generation) = if spec.on_demand {
            let runs = parse_measured(measured, spec.name);
            let first = runs.first().ok_or(format!("{}: no measured.csv row", spec.name))?;
            if runs.iter().any(|r| r.sha256 != first.sha256 || r.bytes != first.bytes) {
                return Err(format!("{}: the platforms disagree on the bytes", spec.name));
            }
            let items: Vec<String> = runs
                .iter()
                .map(|r| {
                    format!(
                        "{{\"platform\": \"{}\", \"host\": \"{}\", \"millis\": {}, \"peak_rss_kib\": {}, \"sha256\": \"{}\"}}",
                        r.platform, r.host, r.millis, r.peak_rss_kib, r.sha256
                    )
                })
                .collect();
            (first.bytes, first.sha256.to_string(), items.join(",\n        "))
        } else {
            let bytes = spec.artifact()?;
            (bytes.len() as u64, hex_digest(&bytes), String::new())
        };
        rows.push(Row {
            name: spec.name.to_string(),
            class: spec.class,
            entities: counts.values().sum(),
            obtain: "generator",
            seed: Some(spec.seed),
            bytes,
            sha256,
            counts,
            bounds: b,
            unit: format!("{:?}", spec.unit()).to_lowercase(),
            evidence: None,
            on_demand: spec.on_demand,
            generation,
        });
    }
    let mut j = String::new();
    j.push_str("{\n  \"schema\": 1,\n");
    let _ = writeln!(j, "  \"generator\": \"{GENERATOR}\",");
    let _ = writeln!(
        j,
        "  \"fixture_revision\": {{\"repo\": \"libviprs/pvec-gold-fixtures\", \"epic\": \"libviprs/pvec-gold-fixtures#1\", \
         \"tag\": \"corpus-r0-open\", \"commit\": \"{FIXTURE_COMMIT}\", \"corpus_revision\": \"r0-open\", \
         \"bundle_sha256\": \"{}\"}},",
        Bundle::vendored().map_err(|e| e.to_string())?.pin().bundle_sha256
    );
    j.push_str("  \"entries\": [\n");
    for (i, r) in rows.iter().enumerate() {
        let counts: Vec<String> = r.counts.iter().map(|(k, v)| format!("\"{k}\": {v}")).collect();
        let _ = write!(
            j,
            "    {{\n      \"name\": \"{}\",\n      \"class\": \"{}\",\n      \"entities\": {},\n      \"obtain\": \"{}\",\n",
            r.name, r.class, r.entities, r.obtain
        );
        if let Some(seed) = r.seed {
            let _ = writeln!(j, "      \"seed\": {seed},");
        }
        let _ = write!(
            j,
            "      \"bytes\": {},\n      \"sha256\": \"{}\",\n      \"counts\": {{{}}},\n      \"bounds\": [{}],\n      \"unit\": \"{}\",\n",
            r.bytes,
            r.sha256,
            counts.join(", "),
            r.bounds.map(hex).join(", "),
            r.unit
        );
        match r.evidence {
            Some(c) => {
                let _ = writeln!(j, "      \"evidence_class\": \"{c}\",");
            }
            None => j.push_str(
                "      \"evidence_class\": null,\n      \"evidence_note\": \"generated by this repository, not an EPIC O fixture\",\n",
            ),
        }
        let (licence, redistribution) = if r.obtain == "generator" {
            ("Apache-2.0 OR MIT, as this repository", "regenerated from the seed, nothing is redistributed")
        } else {
            ("as the r0-open manifest states", "PUBLIC_REDISTRIBUTABLE in the r0-open manifest")
        };
        let _ =
            writeln!(j, "      \"licence\": \"{licence}\",\n      \"redistribution\": \"{redistribution}\",");
        if r.on_demand {
            let _ = writeln!(
                j,
                "      \"on_demand_only\": true,\n      \"generation\": [\n        {}\n      ],",
                r.generation
            );
        }
        let _ = write!(
            j,
            "      \"baseline_eligible\": true\n    }}{}\n",
            if i + 1 < rows.len() { "," } else { "" }
        );
    }
    j.push_str("  ],\n  \"exclusions\": [\n");
    let mut ex: Vec<(String, &str, String)> = refused.into_iter().collect();
    let trial = |s: Shape| {
        let pad = Shape::Line([0.0, 0.0], [10.0, 10.0]);
        artifact(&[s, pad], pvec_container::LengthUnit::Millimetre, "trial", Default::default(), None)
    };
    for (name, shape) in [
        ("pathological-zero-length-line", Shape::Line([3.0, 3.0], [3.0, 3.0])),
        ("pathological-coincident-vertices", Shape::Polyline(vec![[0.0, 0.0], [0.0, 0.0], [1.0, 0.0]])),
    ] {
        match trial(shape) {
            Err(why) => {
                ex.push((name.into(), "WRITER_REFUSAL", format!("the production writer refuses it: {why}")))
            }
            Ok(_) => {
                return Err(format!(
                    "{name}: the writer accepts it now, so it belongs in the pathological class"
                ));
            }
        }
    }
    for class in ["architectural", "mep", "industrial"] {
        ex.push((
            format!("{class}-real-world"),
            "REAL_WORLD_ABSENT",
            "no REAL_WORLD_DIFFERENTIAL entry exists in the fixture repository yet (pvec#100, pvec-gold-fixtures#42), \
             so this class is represented by a modelled drawing only"
                .into(),
        ));
    }
    for (i, (n, k, why)) in ex.iter().enumerate() {
        let why = why.replace('\\', "\\\\").replace('"', "'");
        let _ = writeln!(
            j,
            "    {{\"name\": \"{n}\", \"kind\": \"{k}\", \"reason\": \"{why}\"}}{}",
            if i + 1 < ex.len() { "," } else { "" }
        );
    }
    j.push_str("  ]\n}\n");
    let mut csv = String::from("entry,index,x0,y0,x1,y1\n");
    for r in &rows {
        let v = viewports(&r.name, r.bounds);
        debug_assert_eq!(v.len(), VIEWPORTS);
        for (i, w) in v.iter().enumerate() {
            let _ = writeln!(csv, "{},{i},{},{},{},{}", r.name, w[0], w[1], w[2], w[3]);
        }
    }
    Ok((j, csv))
}
