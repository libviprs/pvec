//! The feature table below capability level (issue 299): `docs/features.toml`, the markdown generated from it
//! and the PAINT.md regions it has to cover. Plain text in, plain text out, like the other checks.

/// One row of `docs/features.toml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    /// `<capability>.<name>` for a feature of the format, `none.<name>` for one the format refuses.
    pub key: String,
    /// One line saying what the feature is.
    pub what: String,
    /// Whether the producer (libviprs) writes it.
    pub producer: bool,
    /// Whether the driver (`pvec-scene`) works it out.
    pub driver: bool,
    /// Whether the viewer (planviewer) draws it.
    pub consumer: bool,
    /// The golden file that exercises it, relative to the repository root. Absent for a refused feature.
    pub golden: Option<String>,
    /// Why the format refuses it. Present exactly for a `none.` feature.
    pub reason: Option<String>,
    /// The `fid-<fixture>/<region>` ids of PAINT.md chapter 7 that depend on it.
    pub regions: Vec<String>,
}

fn unquote(v: &str) -> Result<&str, String> {
    v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).ok_or_else(|| format!("not a string: {v}"))
}

/// Parses `docs/features.toml`: `[[feature]]` tables of `key = value` lines, where a value is a string, a
/// boolean or a one-line array of strings. Anything else is an error naming the line.
pub fn parse_features(text: &str) -> Result<Vec<Feature>, String> {
    let mut out: Vec<Feature> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let at = |m: String| format!("line {}: {m}", n + 1);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[feature]]" {
            out.push(Feature {
                key: String::new(),
                what: String::new(),
                producer: false,
                driver: false,
                consumer: false,
                golden: None,
                reason: None,
                regions: Vec::new(),
            });
            continue;
        }
        let f = out.last_mut().ok_or_else(|| at("a value before the first [[feature]]".into()))?;
        let (k, v) = line.split_once(" = ").ok_or_else(|| at(format!("not `key = value`: {line}")))?;
        let flag = |v: &str| match v {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(at(format!("not a boolean: {v}"))),
        };
        match k {
            "key" => f.key = unquote(v).map_err(at)?.to_string(),
            "what" => f.what = unquote(v).map_err(at)?.to_string(),
            "golden" => f.golden = Some(unquote(v).map_err(at)?.to_string()),
            "reason" => f.reason = Some(unquote(v).map_err(at)?.to_string()),
            "producer" => f.producer = flag(v)?,
            "driver" => f.driver = flag(v)?,
            "consumer" => f.consumer = flag(v)?,
            "regions" => {
                let inner = v
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .ok_or_else(|| at(format!("not an array: {v}")))?;
                for item in inner.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    f.regions.push(unquote(item).map_err(at)?.to_string());
                }
            }
            _ => return Err(at(format!("unknown field {k}"))),
        }
    }
    Ok(out)
}

/// The shape rules: keys are unique and look like `<capability>.<name>`, a feature has a golden or, when its key
/// starts with `none.`, a reason and no golden, and a refused feature is not claimed by any column.
pub fn check_features(features: &[Feature]) -> Result<(), String> {
    for (i, f) in features.iter().enumerate() {
        let k = &f.key;
        if features[..i].iter().any(|g| g.key == *k) {
            return Err(format!("{k}: listed twice"));
        }
        let (head, rest) = k.split_once('.').ok_or_else(|| format!("{k}: not <capability>.<name>"))?;
        if rest.is_empty() || f.what.is_empty() {
            return Err(format!("{k}: needs a name and a `what`"));
        }
        if head == "none" {
            if f.reason.is_none() || f.golden.is_some() || f.producer || f.driver || f.consumer {
                return Err(format!("{k}: a refused feature has a reason, no golden and no column set"));
            }
        } else {
            if head.parse::<u16>().is_err() {
                return Err(format!("{k}: {head} is not a capability number"));
            }
            if f.golden.is_none() {
                return Err(format!("{k}: no golden exercises it"));
            }
            if f.reason.is_some() {
                return Err(format!("{k}: only a refused feature has a reason"));
            }
        }
    }
    Ok(())
}

/// Renders `docs/FEATURES.md`. Deterministic, one trailing newline, compared byte for byte with the committed file.
pub fn render_features_markdown(features: &[Feature]) -> String {
    let yn = |b: bool| if b { "yes" } else { "no" };
    let mut out = String::from("# PVec features\n\n");
    out.push_str(
        "This file is generated from `docs/features.toml`. I do not edit it by hand: the gate regenerates it and \
         fails if the committed file differs by a single byte. To change it I change the toml and run \
         `PVEC_BLESS_FEATURES=1 cargo test -p pvec-checks --test issue_299_features`.\n\n",
    );
    out.push_str(
        "A capability says a stream exists. A feature says which part of it is written (producer), worked out \
         (driver, the `SUPPORTED` list of `pvec-scene`) and drawn (consumer). Every other row has a golden file \
         that exercises it. A `none.` row is something the format refuses on purpose.\n\n",
    );
    out.push_str("| feature | what | producer | driver | consumer | golden |\n|---|---|---|---|---|---|\n");
    for f in features {
        let golden = f.golden.as_deref().map_or_else(|| "none".to_string(), |g| format!("`{g}`"));
        let what = match &f.reason {
            Some(r) => format!("{} Refused: {r}", f.what),
            None => f.what.clone(),
        };
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            f.key,
            what,
            yn(f.producer),
            yn(f.driver),
            yn(f.consumer),
            golden
        ));
    }
    out
}

/// The region ids of the table in `## 7.` of PAINT.md, in order. A cell lists ids separated by commas or
/// semicolons, and an id without its `fid-<fixture>/` prefix takes the one before it.
pub fn paint_regions(paint_md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in paint_md.lines() {
        if line.starts_with("## ") {
            inside = line.starts_with("## 7.");
            continue;
        }
        let cells: Vec<&str> = line.split('|').collect();
        if !inside || cells.len() < 3 || !cells[2].contains("fid-") {
            continue;
        }
        let mut prefix = "";
        for tok in cells[2].split([',', ';']).map(str::trim).filter(|t| !t.is_empty()) {
            if let Some(rest) = tok.strip_prefix("fid-") {
                prefix = &tok[..4 + rest.find('/').map_or(0, |s| s + 1)];
                out.push(tok.to_string());
            } else {
                out.push(format!("{prefix}{tok}"));
            }
        }
    }
    out
}
