//! The `pvec` binary. Usage is in `docs/CLI.md`.

use std::io::Write as _;
use std::panic::catch_unwind;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::panic::set_hook(Box::new(|_| {}));
    match catch_unwind(|| pvec_conformance::run(&args)) {
        Ok((outcome, json)) => {
            let text = if json { outcome.report.render_pretty() } else { outcome.human.clone() };
            let _ = writeln!(std::io::stdout(), "{text}");
            std::process::exit(i32::from(outcome.exit));
        }
        Err(_) => {
            let _ = writeln!(std::io::stderr(), "pvec: internal error");
            std::process::exit(i32::from(pvec_conformance::exit::INTERNAL));
        }
    }
}
