//! Writes every canonical artifact of the vendored corpus to a directory, with `expected.txt` holding
//! `name entity-count` per line, so each prototype is held to a count it did not compute itself.

use pvec_corpus::Bundle;
use pvec_testkit::generate::generate_all;
use std::fmt::Write as _;
use std::fs;

fn main() {
    let dir = std::env::args().nth(1).expect("usage: p74-fixture-gen DIR");
    fs::create_dir_all(&dir).expect("create the directory");
    let bundle = Bundle::vendored().expect("the vendored bundle opens");
    let mut expected = String::new();
    for outcome in generate_all(&bundle).expect("the generator runs") {
        let Ok(artifact) = outcome.result else { continue };
        fs::write(format!("{dir}/{}.pvec", outcome.fixture), &artifact.bytes).expect("write the artifact");
        writeln!(expected, "{} {}", outcome.fixture, artifact.ids.len()).expect("format");
    }
    fs::write(format!("{dir}/expected.txt"), expected).expect("write expected.txt");
}
