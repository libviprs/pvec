//! The versioned report schema, `pvec/conformance-report/1`, and the reader an older consumer uses.
//!
//! Numbers are `{decimal, f64_hex}` scalars like everywhere else in the corpus format. Within a major
//! version a field may be added to the document or to a row, and a consumer ignores what it does not
//! know; a new major version is refused.

use crate::error::{ConformanceError, Result};
use crate::gates::{Gate, GateVerdict, Runner};
use pvec_capabilities::EvidenceClass;
use pvec_corpus::json::{self, Value};
use pvec_corpus::{Scalar, scalar::to_hex};

/// The schema identifier written at the top of every report.
pub const SCHEMA: &str = "pvec/conformance-report/1";

/// One row as a consumer of version 1 reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadRow {
    /// The gate.
    pub gate: Gate,
    /// The fixture id.
    pub fixture: String,
    /// The evidence class.
    pub class: EvidenceClass,
    /// The verdict.
    pub verdict: GateVerdict,
    /// The numbers, by name.
    pub numbers: Vec<(String, f64)>,
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn number(v: f64) -> String {
    if v.is_nan() {
        return r#"{"non_finite": "nan"}"#.to_string();
    }
    if v.is_infinite() {
        return format!(r#"{{"non_finite": "{}"}}"#, if v > 0.0 { "inf" } else { "-inf" });
    }
    let v = v + 0.0;
    format!(r#"{{"decimal": "{v}", "f64_hex": "{}"}}"#, to_hex(v.to_bits()))
}

/// The report as JSON.
pub fn write(runner: &Runner) -> String {
    let rows: Vec<String> = runner
        .rows()
        .iter()
        .map(|r| {
            let numbers: Vec<String> = r
                .numbers
                .iter()
                .map(|(n, v)| format!(r#"{{"name": {}, "value": {}}}"#, quote(n), number(*v)))
                .collect();
            format!(
                r#"    {{"gate": {}, "fixture": {}, "evidence_class": {}, "verdict": "{}", "numbers": [{}]}}"#,
                quote(r.gate.name()),
                quote(&r.fixture),
                quote(r.class.name()),
                match r.verdict {
                    GateVerdict::Pass => "PASS",
                    GateVerdict::Fail => "FAIL",
                    GateVerdict::Routed => "ROUTED",
                },
                numbers.join(", ")
            )
        })
        .collect();
    format!("{{\n  \"schema_version\": \"{SCHEMA}\",\n  \"rows\": [\n{}\n  ]\n}}\n", rows.join(",\n"))
}

fn bad(why: impl Into<String>) -> ConformanceError {
    ConformanceError::BadReport(why.into())
}

fn text<'a>(row: &'a Value, key: &str, at: usize) -> Result<&'a str> {
    row.get(key).and_then(Value::as_str).ok_or_else(|| bad(format!("row {at} has no `{key}`")))
}

fn value(v: &Value, at: &str) -> Result<f64> {
    if let Some(kind) = v.get("non_finite").and_then(Value::as_str) {
        return match kind {
            "nan" => Ok(f64::NAN),
            "inf" => Ok(f64::INFINITY),
            "-inf" => Ok(f64::NEG_INFINITY),
            other => Err(bad(format!("{at}: `{other}` is not a non-finite marker"))),
        };
    }
    Scalar::from_json(at, v).map(|s| s.value()).map_err(|e| bad(e.to_string()))
}

/// Reads a version 1 report, ignoring fields it does not know.
pub fn read(text_in: &str) -> Result<Vec<ReadRow>> {
    let doc = json::parse(text_in.as_bytes()).map_err(|e| bad(e.to_string()))?;
    match doc.get("schema_version").and_then(Value::as_str) {
        Some(SCHEMA) => {}
        Some(other) => {
            return Err(bad(format!("`{other}` is not `{SCHEMA}`; this consumer reads version 1 only")));
        }
        None => return Err(bad("no schema_version")),
    }
    let rows = doc.get("rows").and_then(Value::as_arr).ok_or_else(|| bad("no rows"))?;
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let gate_name = text(row, "gate", i)?;
        let gate = Gate::ALL
            .into_iter()
            .find(|g| g.name() == gate_name)
            .ok_or_else(|| bad(format!("row {i}: unknown gate `{gate_name}`")))?;
        let class_name = text(row, "evidence_class", i)?;
        let class = EvidenceClass::parse(class_name)
            .ok_or_else(|| bad(format!("row {i}: `{class_name}` is not an evidence class")))?;
        let verdict = match text(row, "verdict", i)? {
            "PASS" => GateVerdict::Pass,
            "FAIL" => GateVerdict::Fail,
            "ROUTED" => GateVerdict::Routed,
            other => return Err(bad(format!("row {i}: unknown verdict `{other}`"))),
        };
        let mut numbers = Vec::new();
        for (k, n) in row
            .get("numbers")
            .and_then(Value::as_arr)
            .ok_or_else(|| bad(format!("row {i} has no `numbers`")))?
            .iter()
            .enumerate()
        {
            let name = text(n, "name", i)?;
            let v = n.get("value").ok_or_else(|| bad(format!("row {i} number {k} has no value")))?;
            numbers.push((name.to_string(), value(v, &format!("rows[{i}].numbers[{k}]"))?));
        }
        out.push(ReadRow { gate, fixture: text(row, "fixture", i)?.to_string(), class, verdict, numbers });
    }
    Ok(out)
}
