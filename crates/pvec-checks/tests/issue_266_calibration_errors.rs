//! Issue #266: the rule table of `docs/CALIBRATION.md` is rendered from `CALIBRATION_ERRORS`, the way the
//! `RASTER-NN` table is kept in step, and `FUZZING.md` and `tools/fuzz.sh` know the twelfth target (the thirteenth is `fuzz_fill_paint`, issue 286).

use pvec_checks::repo_root;
use pvec_components::CALIBRATION_ERRORS;
use std::fs;

const BEGIN: &str = "<!-- calibration-errors:begin -->";
const END: &str = "<!-- calibration-errors:end -->";

fn rendered() -> String {
    let mut s = String::from("| rule | error | meaning |\n|---|---|---|\n");
    for (rule, name, meaning) in CALIBRATION_ERRORS {
        s.push_str(&format!("| {rule} | `{name}` | {meaning} |\n"));
    }
    s
}

#[test]
fn the_rule_table_in_calibration_md_is_exactly_the_rendered_one() {
    let path = repo_root().join("docs/CALIBRATION.md");
    let doc = fs::read_to_string(&path).expect("docs/CALIBRATION.md exists");
    let (head, rest) = doc.split_once(BEGIN).expect("CALIBRATION.md has the begin marker of the rule table");
    let (old, tail) = rest.split_once(END).expect("CALIBRATION.md has the end marker of the rule table");
    let want = format!("\n{}", rendered());
    if std::env::var_os("PVEC_BLESS_CALIBRATION").is_some() {
        fs::write(&path, format!("{head}{BEGIN}{want}{END}{tail}")).unwrap();
        return;
    }
    assert!(
        old == want,
        "the rule table of docs/CALIBRATION.md differs from CALIBRATION_ERRORS; regenerate it with \
         PVEC_BLESS_CALIBRATION=1 cargo test -p pvec-checks --test issue_266_calibration_errors"
    );
}

#[test]
fn fuzzing_md_and_the_fuzz_script_name_the_twelfth_target() {
    let doc = fs::read_to_string(repo_root().join("docs/FUZZING.md")).unwrap();
    assert!(doc.contains("The fourteen targets") && doc.contains("`fuzz_calibration` is the twelfth"));
    let script = fs::read_to_string(repo_root().join("tools/fuzz.sh")).unwrap();
    assert!(script.contains("fuzz_calibration"));
    assert!(repo_root().join("fuzz/fuzz_targets/fuzz_calibration.rs").is_file());
}
