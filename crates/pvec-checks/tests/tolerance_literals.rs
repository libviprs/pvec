//! Fails the gate if a numeric tolerance is written anywhere but the policy module (issue P1.2).

use pvec_checks::*;
use std::fs;

/// Finds tolerance-shaped tokens in the code part of one line: exponent-form float literals such as
/// `1e-9` or `2.5E-12`, and any use of `EPSILON`.
fn tolerance_tokens(line: &str) -> Vec<String> {
    // Drop the trailing line comment and the contents of string literals; prose may say 1e-9.
    let code = line.split("//").next().unwrap_or("");
    let mut cleaned = String::new();
    let mut in_str = false;
    for ch in code.chars() {
        if ch == '"' {
            in_str = !in_str;
        } else if !in_str {
            cleaned.push(ch);
        }
    }
    let mut hits = Vec::new();
    if cleaned.contains("EPSILON") {
        hits.push("EPSILON".to_string());
    }
    let b: Vec<char> = cleaned.chars().collect();
    for i in 0..b.len() {
        if (b[i] == 'e' || b[i] == 'E') && i > 0 && b[i - 1].is_ascii_digit() {
            let mut j = i + 1;
            if j < b.len() && (b[j] == '-' || b[j] == '+') {
                j += 1;
            }
            let digits = b[j..].iter().take_while(|c| c.is_ascii_digit()).count();
            // Preceded by an identifier character means a name like `x1e5`; require a clean start.
            let mut s = i;
            while s > 0 && (b[s - 1].is_ascii_digit() || b[s - 1] == '.' || b[s - 1] == '_') {
                s -= 1;
            }
            let ident_before = s > 0 && (b[s - 1].is_alphabetic() || b[s - 1] == '_');
            if digits > 0 && !ident_before {
                hits.push(b[s..j + digits].iter().collect());
            }
        }
    }
    hits
}

#[test]
fn detector_finds_what_it_should() {
    assert_eq!(tolerance_tokens("let t = 1e-9;"), vec!["1e-9"]);
    assert_eq!(tolerance_tokens("x < 2.5E-12 + y"), vec!["2.5E-12"]);
    assert_eq!(tolerance_tokens("(a - b).abs() < f64::EPSILON"), vec!["EPSILON"]);
    assert!(tolerance_tokens("let s = \"1e-9\";").is_empty());
    assert!(tolerance_tokens("let t = 1.0; // 1e-9 here").is_empty());
    assert!(tolerance_tokens("let ee = 3 + e1;").is_empty());
}

#[test]
fn no_tolerance_literal_outside_the_policy_module() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for rel in text_files(&root) {
        let s = rel.to_string_lossy().replace('\\', "/");
        // The detector's own sources, and the one place tolerances are allowed to live.
        if !s.ends_with(".rs")
            || s.starts_with("crates/pvec-checks/")
            || s == "crates/pvec-numeric/src/policy.rs"
        {
            continue;
        }
        let text = fs::read_to_string(root.join(&rel)).unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            for tok in tolerance_tokens(line) {
                offenders.push(format!("{s}:{}: {tok}", n + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "tolerance literals outside crates/pvec-numeric/src/policy.rs:\n{}",
        offenders.join("\n")
    );
}
