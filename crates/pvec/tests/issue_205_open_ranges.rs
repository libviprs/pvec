//! Issue #205: the `http` feature is real. `PvecReader::open_ranges` reads a file through a callback that
//! returns byte ranges, and reads what `PvecReader::open` reads from the same bytes.
#![cfg(feature = "http")]

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_geometry::{Point2, Polyline};
use std::sync::{Arc, Mutex};

fn p(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

fn file() -> Vec<u8> {
    let manifest = Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-205".into(),
        producer_version: "1".into(),
    };
    let mut w = PvecWriter::new(manifest).unwrap();
    for i in 0..40u32 {
        let y = f64::from(i);
        let line = Polyline::new(vec![p(0.0, y), p(10.0, y + 1.0), p(20.0, y)]).unwrap();
        w.push(EntityId::new(i).unwrap(), &Curve::Polyline(line)).unwrap();
    }
    w.finish().unwrap()
}

fn from(
    bytes: Arc<Vec<u8>>,
    log: Arc<Mutex<Vec<(u64, u32)>>>,
) -> impl Fn(u64, u32) -> Result<Vec<u8>, String> {
    move |offset, len| {
        log.lock().unwrap().push((offset, len));
        let end = offset.checked_add(u64::from(len)).filter(|e| *e <= bytes.len() as u64);
        end.map(|e| bytes[offset as usize..e as usize].to_vec()).ok_or_else(|| "out of range".to_string())
    }
}

#[test]
fn issue_205_open_ranges_reads_the_same_entities_as_open() {
    let bytes = file();
    let whole = PvecReader::open(bytes.clone()).unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let ranged =
        PvecReader::open_ranges(bytes.len() as u64, from(Arc::new(bytes), Arc::clone(&log))).unwrap();
    assert_eq!(ranged.manifest(), whole.manifest());
    assert_eq!(ranged.entity_count().unwrap(), whole.entity_count().unwrap());
    assert_eq!(ranged.entities().unwrap(), whole.entities().unwrap());
    assert_eq!(ranged.styles().unwrap(), whole.styles().unwrap());
}

#[test]
fn issue_205_open_ranges_asks_for_the_prefix_first_and_stays_inside_the_file() {
    let bytes = file();
    let n = bytes.len() as u64;
    let log = Arc::new(Mutex::new(Vec::new()));
    let _ranged = PvecReader::open_ranges(n, from(Arc::new(bytes), Arc::clone(&log))).unwrap();
    let asked = log.lock().unwrap().clone();
    assert_eq!(asked[0].0, 0, "the first request is the start of the file");
    assert!(asked.iter().all(|(o, l)| o + u64::from(*l) <= n), "{asked:?}");
    assert!(asked.len() <= 4, "opening took {} requests: {asked:?}", asked.len());
}

#[test]
fn issue_205_a_callback_that_fails_or_answers_short_is_an_error_not_a_panic() {
    let bytes = file();
    let n = bytes.len() as u64;
    assert!(PvecReader::open_ranges(n, |_, _| Err("network down".to_string())).is_err());
    assert!(PvecReader::open_ranges(n, |_, len| Ok(vec![0u8; (len as usize).saturating_sub(1)])).is_err());
    assert!(PvecReader::open_ranges(n, |_, len| Ok(vec![0u8; len as usize])).is_err());
    assert!(PvecReader::open_ranges(0, |_, _| Ok(Vec::new())).is_err());
}

#[test]
fn issue_205_the_reader_stays_send_and_sync_with_a_callback() {
    fn is<T: Send + Sync>() {}
    is::<PvecReader>();
}
