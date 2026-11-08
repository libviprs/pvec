//! The `pvec` command line tool (issue P7.7): the checks of phases 2 to 6 from a shell.
//!
//! Every subcommand returns an [`Outcome`]: an exit code, a JSON report with a `report_schema_version`
//! (schema: `docs/cli-report.schema.json`) and a human-readable text. `docs/CLI.md` documents them all.

pub mod commands;
pub mod decode;
pub mod json;

use json::Json;

/// The version of the JSON report. It goes up when a field is removed or changes meaning.
pub const REPORT_SCHEMA_VERSION: u64 = 1;

/// The exit-code contract.
pub mod exit {
    /// The file conforms.
    pub const CONFORMS: u8 = 0;
    /// Non-conformance was found: a comparison disagreed.
    pub const NONCONFORMING: u8 = 1;
    /// The file was refused or is invalid.
    pub const REFUSED: u8 = 2;
    /// The file requires a capability this build does not support.
    pub const UNSUPPORTED_CAPABILITY: u8 = 3;
    /// The command line was wrong.
    pub const USAGE: u8 = 64;
    /// A file could not be read.
    pub const IO: u8 = 65;
    /// The tool itself failed.
    pub const INTERNAL: u8 = 70;
}

/// What a subcommand produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The exit code.
    pub exit: u8,
    /// The JSON report.
    pub report: Json,
    /// The same, for a person.
    pub human: String,
    /// The named error code when the file was refused (`DIR-06`, `LIMIT-01`, `INTEGRITY-03`, ...).
    pub error_code: Option<String>,
}

const USAGE: &str = "usage: pvec <subcommand> [--format json|human] <args>
  inspect <file.pvec>
  validate <file.pvec>
  dump-manifest <file.pvec>
  capabilities <file.pvec>
  compare-step <file.pvec> <file.step> --evaluator-report <report.txt> [--requests]
  tessellate <file.pvec> --error-px <n> [--units-per-pixel <n>]
  bench-open <file.pvec> [--iterations <n>]
  calibrate <file.pvec> (--remove | --block <calibration.bin> | --scale <m> [--provenance manual|declared_unit|georeference]
            | --plan-scale <n>:<d> --paper-unit <m> | --measured <x0,y0,x1,y1> --distance <m>) [--note <text>]
exit codes: 0 conforms, 1 non-conformance found, 2 refused or invalid, 3 unsupported required capability, 64 usage, 65 io, 70 internal";

fn usage(message: &str) -> (Outcome, bool) {
    (
        Outcome {
            exit: exit::USAGE,
            report: Json::Null,
            human: format!("pvec: {message}\n{USAGE}"),
            error_code: None,
        },
        false,
    )
}

/// Runs the tool on `args` (without the program name), reading the files they name. The flag is whether
/// the report is to be printed as JSON.
pub fn run(args: &[String]) -> (Outcome, bool) {
    let Some((command, rest)) = args.split_first() else { return usage("no subcommand") };
    let mut json = false;
    let mut positional: Vec<&str> = Vec::new();
    let (mut report, mut error_px, mut units_per_px, mut iterations, mut want_requests) =
        (None, None, 1.0f64, 10u32, false);
    // the flags of `calibrate`, read once the subcommand is known
    let mut calibrate: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().map(String::as_str).ok_or(format!("{name} needs a value"));
        let parsed: Result<(), String> = match a.as_str() {
            "--format" => value("--format").and_then(|v| match v {
                "json" => {
                    json = true;
                    Ok(())
                }
                "human" => Ok(()),
                other => Err(format!("--format is json or human, not {other}")),
            }),
            "--evaluator-report" => value("--evaluator-report").map(|v| report = Some(v.to_string())),
            "--requests" => {
                want_requests = true;
                Ok(())
            }
            "--error-px" => value("--error-px").and_then(|v| match v.parse::<f64>() {
                Ok(x) if x.is_finite() && x > 0.0 => {
                    error_px = Some(x);
                    Ok(())
                }
                _ => Err(format!("--error-px is a positive number, not {v}")),
            }),
            "--units-per-pixel" => value("--units-per-pixel").and_then(|v| match v.parse::<f64>() {
                Ok(x) if x.is_finite() && x > 0.0 => {
                    units_per_px = x;
                    Ok(())
                }
                _ => Err(format!("--units-per-pixel is a positive number, not {v}")),
            }),
            "--iterations" => value("--iterations").and_then(|v| match v.parse::<u32>() {
                Ok(n) if (1..=100_000).contains(&n) => {
                    iterations = n;
                    Ok(())
                }
                _ => Err(format!("--iterations is 1 to 100000, not {v}")),
            }),
            "--remove" => {
                calibrate.insert("--remove", String::new());
                Ok(())
            }
            flag @ ("--block" | "--scale" | "--provenance" | "--plan-scale" | "--paper-unit"
            | "--measured" | "--distance" | "--note") => value(flag).map(|v| {
                calibrate.insert(flag, v.to_string());
            }),
            flag if flag.starts_with("--") => Err(format!("unknown option {flag}")),
            file => {
                positional.push(file);
                Ok(())
            }
        };
        if let Err(e) = parsed {
            return usage(&e);
        }
    }
    let wanted = match command.as_str() {
        "compare-step" => 2,
        "inspect" | "validate" | "dump-manifest" | "capabilities" | "tessellate" | "bench-open"
        | "calibrate" => 1,
        other => return usage(&format!("unknown subcommand {other}")),
    };
    if positional.len() != wanted {
        return usage(&format!("{command} takes {wanted} file argument(s), not {}", positional.len()));
    }
    let read = |path: &str| std::fs::read(path).map_err(|e| format!("pvec: cannot read {path}: {e}\n"));
    let io = |human: String| (Outcome { exit: exit::IO, report: Json::Null, human, error_code: None }, false);
    let path = positional[0];
    let bytes = match read(path) {
        Ok(b) => b,
        Err(h) => return io(h),
    };
    let outcome = match command.as_str() {
        "inspect" => commands::inspect(&bytes, path),
        "validate" => commands::validate(&bytes, path),
        "dump-manifest" => commands::dump_manifest(&bytes, path),
        "capabilities" => commands::capabilities(&bytes, path),
        "bench-open" => commands::bench_open(&bytes, path, iterations),
        "calibrate" => {
            // the scale the flags state, checked as a command line and not yet as a calibration: the CAL rules are
            // the writer's, and a calibration they refuse is a refused file (exit 2), not a usage error
            let number = |name: &str| -> Result<Option<f64>, String> {
                calibrate.get(name).map_or(Ok(None), |v| match v.parse::<f64>() {
                    Ok(x) if x.is_finite() && x > 0.0 => Ok(Some(x)),
                    _ => Err(format!("{name} is a positive number, not {v}")),
                })
            };
            let note = calibrate.get("--note").cloned().unwrap_or_default();
            let scale = (|| -> Result<Option<pvec_components::ScaleClaim>, String> {
                use pvec_components::{Provenance, ScaleClaim};
                let claim = |metres_per_source_unit, provenance| ScaleClaim {
                    metres_per_source_unit,
                    provenance,
                    uncertainty: None,
                    note: note.clone(),
                };
                if let Some(ratio) = calibrate.get("--plan-scale") {
                    let paper = number("--paper-unit")?.ok_or("--plan-scale needs --paper-unit")?;
                    let parsed =
                        ratio.split_once(':').and_then(|(n, d)| Some((n.parse().ok()?, d.parse().ok()?)));
                    let (numerator, denominator): (u32, u32) =
                        parsed.ok_or(format!("--plan-scale is <n>:<d>, not {ratio}"))?;
                    let metres = paper * f64::from(denominator) / f64::from(numerator);
                    let provenance =
                        Provenance::PlanScale { numerator, denominator, paper_metres_per_unit: paper };
                    return Ok(Some(claim(metres, provenance)));
                }
                if let Some(points) = calibrate.get("--measured") {
                    let distance = number("--distance")?.ok_or("--measured needs --distance")?;
                    let p: Vec<f64> = points.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    let [x0, y0, x1, y1] = p[..] else {
                        return Err(format!("--measured is <x0,y0,x1,y1>, not {points}"));
                    };
                    let length = ((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)).sqrt();
                    // coincident points leave the factor at the distance, so CAL-06 is what refuses them
                    let metres = if length > 0.0 { distance / length } else { distance };
                    let provenance =
                        Provenance::Measured { p0: [x0, y0], p1: [x1, y1], distance_metres: distance };
                    return Ok(Some(claim(metres, provenance)));
                }
                let Some(metres) = number("--scale")? else { return Ok(None) };
                let provenance = match calibrate.get("--provenance").map_or("manual", String::as_str) {
                    "manual" => Provenance::Manual,
                    "declared_unit" => Provenance::DeclaredUnit,
                    "georeference" => Provenance::Georeference,
                    other => {
                        return Err(format!(
                            "--provenance is manual, declared_unit or georeference, not {other}"
                        ));
                    }
                };
                Ok(Some(claim(metres, provenance)))
            })();
            let scale = match scale {
                Ok(s) => s,
                Err(e) => return usage(&e),
            };
            let block = match calibrate.get("--block").map(|p| read(p)) {
                Some(Ok(b)) => Some(b),
                Some(Err(h)) => return io(h),
                None => None,
            };
            let remove = calibrate.contains_key("--remove");
            if !remove && scale.is_none() && block.is_none() {
                return usage("calibrate needs --remove, --block or a scale");
            }
            let (outcome, written) = commands::calibrate(&bytes, path, remove, block.as_deref(), scale);
            if let Some(new) = written {
                // the new file goes next to the old one and is renamed over it, so a reader sees one or the other
                let temp = format!("{path}.calibrate-{}.tmp", std::process::id());
                let done = std::fs::write(&temp, &new).and_then(|()| std::fs::rename(&temp, path));
                if let Err(e) = done {
                    let _ = std::fs::remove_file(&temp);
                    return io(format!("pvec: cannot write {path}: {e}\n"));
                }
            }
            outcome
        }
        "tessellate" => match error_px {
            Some(px) => commands::tessellate(&bytes, path, px, units_per_px),
            None => return usage("tessellate needs --error-px"),
        },
        _ => {
            let Some(report_path) = report else { return usage("compare-step needs --evaluator-report") };
            let (step, text) = match (read(positional[1]), read(&report_path)) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(h), _) | (_, Err(h)) => return io(h),
            };
            commands::compare_step(
                &bytes,
                path,
                positional[1],
                &step,
                &String::from_utf8_lossy(&text),
                want_requests,
            )
        }
    };
    (outcome, json)
}
