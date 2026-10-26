//! Shared by the module-level checks: a gate module writes no numeric tolerance of its own.

/// Panics when `src/<file>` has an exponent-form float literal, `EPSILON`, a small decimal literal,
/// or does not call the numeric policy. Hex literals such as `0x9E37` are not floats.
pub fn assert_no_tolerance_literal(file: &str) {
    let path = format!("{}/src/{file}", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap();
    let code: String =
        src.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    assert!(code.contains("Policy::V1"), "{file}: comparisons go through the numeric policy");
    assert!(!code.contains("EPSILON") && !code.contains("0.000"), "{file}: a tolerance-shaped constant");
    let b: Vec<char> = code.chars().collect();
    for i in 1..b.len() {
        let exponent = (b[i] == 'e' || b[i] == 'E')
            && b[i - 1].is_ascii_digit()
            && b.get(i + 1).is_some_and(|c| *c == '-' || c.is_ascii_digit());
        if !exponent {
            continue;
        }
        let mut start = i;
        while start > 0
            && (b[start - 1].is_ascii_alphanumeric() || b[start - 1] == '_' || b[start - 1] == '.')
        {
            start -= 1;
        }
        let token: String = b[start..i].iter().collect();
        assert!(token.starts_with("0x"), "{file}: an exponent-form literal near `{token}{}`", b[i]);
    }
}
