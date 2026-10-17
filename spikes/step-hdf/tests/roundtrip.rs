//! The two HDF5 layouts must give back the fixture: exactly for H2, and to within the rounding
//! that STEP's origin-plus-vector line form loses for H1. Viewport reads must return exactly the
//! entities whose bounds overlap the viewport.

use pvec_fixtures::{Drawing, generate, overlaps, viewport_script};
use step_hdf_spike::util::*;
use step_hdf_spike::{h1, h2};

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("step-hdf-spike-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().into_owned()
}

fn drawing() -> Drawing {
    generate::small_synthetic()
}

#[test]
fn h2_round_trips_bit_exactly_in_every_configuration() {
    let d = drawing();
    for cfg in configs() {
        let path = tmp(&format!("h2-{}.h5", cfg.name.replace([' ', '+'], "_")));
        h2::write(&d, &path, &cfg).unwrap();
        let got = h2::read_all(&path).unwrap();
        let rebuilt = Drawing { name: d.name, extent: d.extent, entities: got };
        assert_eq!(rebuilt.checksum(), d.checksum(), "{}", cfg.name);
    }
}

#[test]
fn h1_round_trips_within_the_rounding_of_the_step_line_form() {
    let d = drawing();
    let path = tmp("h1.h5");
    h1::write(&d, &path, &Cfg::CONTIGUOUS).unwrap();
    let got = h1::read_all(&path).unwrap();
    assert_eq!(got.len(), d.entities.len());
    let mut worst = 0.0f64;
    let mut inexact = 0;
    for (a, b) in d.entities.iter().zip(&got) {
        let dev = max_deviation(&a.geom, &b.geom).expect("same shape");
        worst = worst.max(dev);
        inexact += usize::from(dev != 0.0);
    }
    // Lines and ellipse placements lose the last bits (end point = origin + length * direction).
    let tol = pvec_numeric::Policy::V1.tol(d.extent[2], d.extent[2]);
    assert!(worst < tol, "worst deviation {worst} against tolerance {tol}");
    assert!(inexact > 0, "if every entity were exact the note about STEP's line form would be wrong");
}

#[test]
fn viewport_reads_return_exactly_the_overlapping_entities() {
    let d = drawing();
    let (p1, p2) = (tmp("vp-h1.h5"), tmp("vp-h2.h5"));
    h1::write(&d, &p1, &Cfg::CONTIGUOUS).unwrap();
    h2::write(&d, &p2, &Cfg::CONTIGUOUS).unwrap();
    for s in viewport_script() {
        let rect = d.viewport(&s);
        let want: Vec<u32> =
            d.entities.iter().filter(|e| overlaps(&e.geom.bbox(), &rect)).map(|e| e.id).collect();
        let a: Vec<u32> = h1::read_viewport(&p1, None, rect).unwrap().iter().map(|e| e.id).collect();
        let b: Vec<u32> = h2::read_viewport(&p2, None, rect).unwrap().iter().map(|e| e.id).collect();
        assert_eq!(a, want, "h1, {}", s.name);
        assert_eq!(b, want, "h2, {}", s.name);
    }
}

#[test]
fn the_read_log_is_parsed_and_merging_is_sound() {
    let d = drawing();
    let path = tmp("log.h5");
    let log = tmp("log.txt");
    h2::write(&d, &path, &Cfg::CONTIGUOUS).unwrap();
    let _ = std::fs::remove_file(&log);
    let rect = d.viewport(&viewport_script()[1]);
    h2::read_viewport(&path, Some(&log), rect).unwrap();
    let io = Io::parse(&std::fs::read_to_string(&log).unwrap());
    assert!(!io.reads.is_empty(), "the log driver recorded no reads");
    let (r0, b0) = io.merged(0);
    let (r1, b1) = io.merged(64 * 1024);
    assert!(r1 <= r0 && r0 <= io.reads.len());
    assert!(b1 >= b0, "merging with a gap can only add bytes");
    assert!(io.max_addr() <= std::fs::metadata(&path).unwrap().len());
}

#[test]
fn merged_ranges_handle_overlap_adjacency_and_gaps() {
    let io = Io { reads: vec![(100, 10), (110, 10), (125, 5), (105, 10), (1000, 1)] };
    assert_eq!(io.merged(0), (3, 20 + 5 + 1));
    assert_eq!(io.merged(5), (2, 30 + 1));
    assert_eq!(io.bytes(), 36);
}

#[test]
fn a_truncated_file_is_an_error_from_the_library_not_silently_wrong() {
    let d = drawing();
    let path = tmp("trunc.h5");
    h2::write(&d, &path, &Cfg::CONTIGUOUS).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let cut = tmp("trunc-cut.h5");
    std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
    // The spike's readers `expect` on library errors, so a damaged file panics here. The point of the
    // test is that it cannot return a plausible drawing: any outcome other than a failure is a bug.
    let outcome = std::panic::catch_unwind(|| h2::read_all(&cut));
    assert!(matches!(outcome, Err(_) | Ok(Err(_))), "a truncated file decoded without error");
}
