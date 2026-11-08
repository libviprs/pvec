//! Issue #266 (#263 slice 2): the kind 11 calibration block of `docs/CALIBRATION.md`, encoded and parsed. The
//! layout is held byte for byte, a block that parses is the block that encodes, and each `CAL-NN` rule refuses
//! the calibration with its own rule.

use pvec_components::{
    CALIBRATION_ERRORS, Calibration, CalibrationError, ControlPoint, CrsDefinition, CrsKind, DefinitionFormat,
    FitMethod, Georeference, MAX_CALIBRATION_BYTES, MAX_CALIBRATION_POINTS, Provenance, ScaleClaim,
    check_residual, decode_calibration, encode_calibration,
};

const HEADER: usize = 16;
const SCALE: usize = 80;
const GEO: usize = 72;

fn plan_scale() -> Calibration {
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 0.0254 / 72.0 * 100.0,
            provenance: Provenance::PlanScale {
                numerator: 1,
                denominator: 100,
                paper_metres_per_unit: 0.0254 / 72.0,
            },
            uncertainty: None,
            note: "1:100".to_string(),
        }),
        georeference: None,
        external_id: String::new(),
    }
}

fn measured() -> Calibration {
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 0.001,
            provenance: Provenance::Measured { p0: [0.0, 0.0], p1: [3000.0, 4000.0], distance_metres: 5.0 },
            uncertainty: Some(0.01),
            note: String::new(),
        }),
        georeference: None,
        external_id: "cal-7".to_string(),
    }
}

/// A GeoPDF-shaped calibration: four corners in page points to longitude and latitude, fitted, with a WKT2
/// definition and a scale derived from the georeference.
fn geopdf() -> Calibration {
    let corners = [[0.0, 0.0], [612.0, 0.0], [612.0, 792.0], [0.0, 792.0]];
    let m = [0.0001, 0.0, 0.0, 0.0001, -1.5, 51.25];
    let control_points: Vec<ControlPoint> = corners
        .iter()
        .map(|&[x, y]| ControlPoint { source: [x, y], world: [m[0] * x + m[1] * y + m[4], m[2] * x + m[3] * y + m[5]] })
        .collect();
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 6.95,
            provenance: Provenance::Georeference,
            uncertainty: Some(0.002),
            note: "derived geodesically from two /GPTS".to_string(),
        }),
        georeference: Some(Georeference {
            source_to_crs: m,
            crs_kind: CrsKind::Geographic,
            method: FitMethod::Fitted,
            crs_id: "EPSG:4326".to_string(),
            definition: Some(CrsDefinition {
                format: DefinitionFormat::Wkt2,
                text: "GEOGCRS[\"WGS 84\",\n\tDATUM[\"World Geodetic System 1984\"]]".to_string(),
            }),
            control_points,
            residual_max: 0.0,
        }),
        external_id: String::new(),
    }
}

fn rule(bytes: &[u8]) -> &'static str {
    match decode_calibration(bytes) {
        Ok(_) => "ok",
        Err(e) => e.rule(),
    }
}

fn encode_rule(c: &Calibration) -> &'static str {
    match encode_calibration(c) {
        Ok(_) => "ok",
        Err(e) => e.rule(),
    }
}

fn put_f64(bytes: &mut [u8], at: usize, v: f64) {
    bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn the_header_and_the_scale_record_are_where_the_spec_puts_them() {
    let c = plan_scale();
    let b = encode_calibration(&c).unwrap();
    assert_eq!(&b[0..4], b"PVCL");
    assert_eq!(u16::from_le_bytes([b[4], b[5]]), 1, "version");
    assert_eq!(u16::from_le_bytes([b[6], b[7]]), 1, "flags: a scale record only");
    assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize, b.len(), "length");
    assert_eq!(b[12], 0, "external id length");
    assert_eq!(&b[13..16], &[0, 0, 0]);
    assert_eq!(b.len(), HEADER + SCALE + "1:100".len());
    let f = |at: usize| f64::from_le_bytes(b[HEADER + at..HEADER + at + 8].try_into().unwrap());
    assert_eq!(f(0).to_bits(), (0.0254f64 / 72.0 * 100.0).to_bits());
    assert_eq!(f(16).to_bits(), (0.0254f64 / 72.0).to_bits());
    assert_eq!(u32::from_le_bytes(b[HEADER + 24..HEADER + 28].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(b[HEADER + 28..HEADER + 32].try_into().unwrap()), 100);
    assert_eq!((b[HEADER + 72], b[HEADER + 73], b[HEADER + 74]), (2, 0, 5), "provenance, has_uncertainty, note_len");
    assert_eq!(&b[HEADER + 75..HEADER + 80], &[0; 5]);
    assert_eq!(&b[HEADER + SCALE..], b"1:100");
}

#[test]
fn the_georeference_record_and_the_tail_are_in_the_spec_order() {
    let c = geopdf();
    let b = encode_calibration(&c).unwrap();
    let g = c.georeference.as_ref().unwrap();
    let def = &g.definition.as_ref().unwrap().text;
    let note = &c.scale.as_ref().unwrap().note;
    assert_eq!(u16::from_le_bytes([b[6], b[7]]), 3, "both records");
    let at = HEADER + SCALE;
    for (i, x) in g.source_to_crs.iter().enumerate() {
        assert_eq!(b[at + 8 * i..at + 8 * i + 8], x.to_le_bytes());
    }
    assert_eq!(u32::from_le_bytes(b[at + 56..at + 60].try_into().unwrap()) as usize, def.len());
    assert_eq!(u16::from_le_bytes([b[at + 60], b[at + 61]]), 4, "point count");
    assert_eq!((b[at + 62], b[at + 63], b[at + 64], b[at + 65]), (2, 2, 9, 1), "kind, method, id len, format");
    assert_eq!(&b[at + 66..at + 72], &[0; 6]);
    let tail = at + GEO;
    let p0: Vec<f64> =
        (0..4).map(|k| f64::from_le_bytes(b[tail + 8 * k..tail + 8 * k + 8].try_into().unwrap())).collect();
    let cp = &g.control_points[0];
    assert_eq!(p0, [cp.source[0], cp.source[1], cp.world[0], cp.world[1]]);
    let strings = tail + 4 * 32;
    let expected = [g.crs_id.as_bytes(), def.as_bytes(), note.as_bytes(), c.external_id.as_bytes()].concat();
    assert_eq!(&b[strings..], &expected[..]);
}

#[test]
fn every_fixture_round_trips_and_a_block_that_parses_is_the_block_that_encodes() {
    for c in [plan_scale(), measured(), geopdf()] {
        let b = encode_calibration(&c).unwrap();
        let back = decode_calibration(&b).unwrap();
        assert_eq!(back, c);
        assert_eq!(encode_calibration(&back).unwrap(), b);
        assert_eq!(c.records(), u32::from(c.scale.is_some()) + u32::from(c.georeference.is_some()));
    }
}

#[test]
fn the_largest_block_the_rules_allow_is_49_894_bytes_and_under_limit_17() {
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    g.control_points = (0..MAX_CALIBRATION_POINTS)
        .map(|i| {
            let (x, y) = ((i % 32) as f64, (i / 32) as f64);
            ControlPoint { source: [x, y], world: [0.0001 * x - 1.5, 0.0001 * y + 51.25] }
        })
        .collect();
    g.crs_id = "urn:ogc:def:crs:EPSG::27700".repeat(3)[..64].to_string();
    g.definition = Some(CrsDefinition { format: DefinitionFormat::Projjson, text: "x".repeat(16_384) });
    c.scale.as_mut().unwrap().note = "n".repeat(255);
    c.external_id = "e".repeat(255);
    let b = encode_calibration(&c).unwrap();
    assert_eq!(b.len(), 49_894);
    assert_eq!(MAX_CALIBRATION_BYTES, 65_536);
    assert!(b.len() <= MAX_CALIBRATION_BYTES as usize);
    assert_eq!(decode_calibration(&b).unwrap(), c);
}

#[test]
fn limit_17_refuses_a_block_over_the_ceiling_before_it_is_parsed() {
    let mut b = encode_calibration(&plan_scale()).unwrap();
    b.resize(MAX_CALIBRATION_BYTES as usize + 1, 0);
    assert_eq!(rule(&b), "LIMIT-17");
}

#[test]
fn cal_01_magic_version_flags_and_reserved() {
    let b = encode_calibration(&plan_scale()).unwrap();
    for (at, v) in [(0, b'Q'), (4, 2), (6, 1 | 4), (7, 1), (13, 1), (15, 1), (HEADER + 79, 1)] {
        let mut m = b.clone();
        m[at] = if at == 6 { v } else { m[at] ^ v };
        assert_eq!(rule(&m), "CAL-01", "byte {at}");
    }
}

#[test]
fn cal_02_the_length_and_the_bytes_must_agree_and_be_consumed() {
    let b = encode_calibration(&measured()).unwrap();
    let mut m = b.clone();
    m[8] ^= 1;
    assert_eq!(rule(&m), "CAL-02", "length field");
    assert_eq!(rule(&b[..b.len() - 1]), "CAL-02", "truncated");
    assert_eq!(rule(&b[..10]), "CAL-02", "shorter than the header");
    let mut m = b.clone();
    m.push(0);
    let n = m.len() as u32;
    m[8..12].copy_from_slice(&n.to_le_bytes());
    assert_eq!(rule(&m), "CAL-02", "a byte left over");
}

#[test]
fn cal_03_counts_and_lengths_over_their_caps() {
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    g.control_points = (0..=MAX_CALIBRATION_POINTS)
        .map(|i| ControlPoint { source: [i as f64, 0.0], world: [i as f64, 0.0] })
        .collect();
    g.method = FitMethod::Declared;
    assert_eq!(encode_rule(&c), "CAL-03");
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().crs_id = "E".repeat(65);
    assert_eq!(encode_rule(&c), "CAL-03");
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().definition =
        Some(CrsDefinition { format: DefinitionFormat::Wkt2, text: "x".repeat(16_385) });
    assert_eq!(encode_rule(&c), "CAL-03");
    let mut c = plan_scale();
    c.external_id = "e".repeat(256);
    assert_eq!(encode_rule(&c), "CAL-03");
    // the point count field, raised past the cap, is refused before anything is sized from it
    let mut b = encode_calibration(&geopdf()).unwrap();
    b[HEADER + SCALE + 60..HEADER + SCALE + 62].copy_from_slice(&1025u16.to_le_bytes());
    assert_eq!(rule(&b), "CAL-03");
}

#[test]
fn cal_04_numbers_out_of_range_non_finite_or_negative_zero() {
    let b = encode_calibration(&measured()).unwrap();
    for (at, v) in [
        (HEADER, f64::NAN),
        (HEADER, -0.0),
        (HEADER, (1u64 << 41) as f64),
        (HEADER, 1.0 / (1u64 << 41) as f64),
        (HEADER + 8, 1.0),
        (HEADER + 8, -0.25),
        (HEADER + 32, f64::INFINITY),
        (HEADER + 64, 0.0),
    ] {
        let mut m = b.clone();
        put_f64(&mut m, at, v);
        assert_eq!(rule(&m), "CAL-04", "offset {at} value {v}");
    }
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().control_points[1].world[0] = f64::NAN;
    assert_eq!(encode_rule(&c), "CAL-04");
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    g.residual_max = -1.0;
    assert_eq!(encode_rule(&c), "CAL-04");
}

#[test]
fn cal_05_a_singular_affine_or_a_linear_entry_past_2_20() {
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().source_to_crs = [1.0, 2.0, 2.0, 4.0, 0.0, 0.0];
    assert_eq!(encode_rule(&c), "CAL-05");
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().source_to_crs = [(1u64 << 21) as f64, 0.0, 0.0, 1.0, 0.0, 0.0];
    assert_eq!(encode_rule(&c), "CAL-05");
}

#[test]
fn cal_06_contradictory_control_points_or_a_degenerate_measured_pair() {
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    let mut twin = g.control_points[0];
    twin.world[0] += 1.0;
    g.control_points.push(twin);
    assert_eq!(encode_rule(&c), "CAL-06");
    // an exact duplicate is not a contradiction
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    let twin = g.control_points[0];
    g.control_points.push(twin);
    assert_eq!(encode_rule(&c), "ok");
    let mut c = measured();
    c.scale.as_mut().unwrap().provenance =
        Provenance::Measured { p0: [5.0, 5.0], p1: [5.0, 5.0], distance_metres: 1.0 };
    assert_eq!(encode_rule(&c), "CAL-06");
}

#[test]
fn cal_07_text_and_the_id_character_class() {
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().crs_id = "EPSG 27700".to_string();
    assert_eq!(encode_rule(&c), "CAL-07");
    for bad in ["a\u{202E}b", "a\u{2028}", "a\nb", "a\u{0085}b", "a\tb"] {
        let mut c = plan_scale();
        c.scale.as_mut().unwrap().note = bad.to_string();
        assert_eq!(encode_rule(&c), "CAL-07", "{bad:?}");
    }
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().definition =
        Some(CrsDefinition { format: DefinitionFormat::Wkt2, text: "a\r\nb".to_string() });
    assert_eq!(encode_rule(&c), "CAL-07");
    // bytes that are not UTF-8
    let mut b = encode_calibration(&plan_scale()).unwrap();
    let n = b.len();
    b[n - 1] = 0xFF;
    assert_eq!(rule(&b), "CAL-07");
}

#[test]
fn cal_08_undefined_tags() {
    let b = encode_calibration(&geopdf()).unwrap();
    let g = HEADER + SCALE;
    for (at, v) in [(HEADER + 72, 0), (HEADER + 72, 6), (HEADER + 73, 2), (g + 62, 4), (g + 63, 0), (g + 65, 3), (g + 65, 0)] {
        let mut m = b.clone();
        m[at] = v;
        assert_eq!(rule(&m), "CAL-08", "byte {at} = {v}");
    }
}

#[test]
fn cal_09_nothing_present() {
    let c = Calibration { scale: None, georeference: None, external_id: "x".to_string() };
    assert_eq!(encode_rule(&c), "CAL-09");
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    g.crs_id.clear();
    g.definition = None;
    assert_eq!(encode_rule(&c), "CAL-09");
    let mut b = encode_calibration(&plan_scale()).unwrap();
    b[6] = 0;
    // with no record the scale bytes are left over, but the empty flags are what is wrong
    assert_eq!(rule(&b), "CAL-09");
}

#[test]
fn cal_10_evidence_that_does_not_fit_its_provenance() {
    let mut c = plan_scale();
    c.scale.as_mut().unwrap().provenance =
        Provenance::PlanScale { numerator: 0, denominator: 100, paper_metres_per_unit: 0.0254 / 72.0 };
    assert_eq!(encode_rule(&c), "CAL-10");
    let mut c = plan_scale();
    c.scale.as_mut().unwrap().provenance = Provenance::Georeference;
    assert_eq!(encode_rule(&c), "CAL-10", "georeference provenance with no georeference");
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    g.method = FitMethod::Declared;
    g.residual_max = 0.5;
    assert_eq!(encode_rule(&c), "CAL-10", "a residual on a declared record");
    let mut c = geopdf();
    c.georeference.as_mut().unwrap().control_points.truncate(1);
    assert_eq!(encode_rule(&c), "CAL-10", "fitted from one point");
    // evidence bytes set for a provenance that does not use them, and an uncertainty without its flag
    let b = encode_calibration(&plan_scale()).unwrap();
    let mut m = b.clone();
    put_f64(&mut m, HEADER + 64, 2.0);
    assert_eq!(rule(&m), "CAL-10");
    let mut m = b.clone();
    put_f64(&mut m, HEADER + 8, 0.5);
    assert_eq!(rule(&m), "CAL-10");
}

#[test]
fn cal_11_the_residual_is_recomputed_the_same_way_everywhere() {
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    g.control_points[2].world[0] += 0.003;
    let recomputed = check_residual(g).expect_err("a stored zero against a real residual");
    assert_eq!(recomputed.rule(), "CAL-11");
    let CalibrationError::ResidualMismatch { recomputed, .. } = recomputed else { panic!() };
    g.residual_max = recomputed;
    assert_eq!(check_residual(g), Ok(recomputed));
    assert!(encode_calibration(&c).is_ok(), "the writer does not run CAL-11");
    // a declared record has no residual to check
    let mut c = geopdf();
    let g = c.georeference.as_mut().unwrap();
    g.method = FitMethod::Declared;
    g.control_points[2].world[0] += 5.0;
    assert_eq!(check_residual(g), Ok(0.0));
}

#[test]
fn the_error_table_has_one_row_per_rule_in_order() {
    let rules: Vec<&str> = CALIBRATION_ERRORS.iter().map(|(r, _, _)| *r).collect();
    let mut want = vec!["LIMIT-17".to_string()];
    want.extend((1..=12).map(|n| format!("CAL-{n:02}")));
    assert_eq!(rules, want);
}
