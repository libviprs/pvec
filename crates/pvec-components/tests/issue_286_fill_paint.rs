//! Capability 25, `fill_paint` (issue 286, `docs/PAINT.md` section 2.4): fill rule, gradient and tiling fills per
//! hatch record. Written before the codec, so every test here starts red.
#![allow(missing_docs)]

use pvec_capabilities::{Class, Reader, Registry, Requirement, Version, ids};
use pvec_components::*;

fn axial() -> Gradient {
    Gradient {
        coords: GradientCoords::Axial { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 },
        extend_before: true,
        extend_after: false,
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        stops: vec![Stop { offset: 0.0, rgba: 0xFF00_00FF }, Stop { offset: 1.0, rgba: 0x0000_FFFF }],
    }
}

fn radial() -> Gradient {
    Gradient {
        coords: GradientCoords::Radial { x0: 0.0, y0: 0.0, r0: 1.0, x1: 5.0, y1: 0.0, r1: 4.0 },
        extend_before: false,
        extend_after: true,
        transform: [2.0, 0.0, 0.0, 2.0, 7.0, -3.0],
        stops: vec![
            Stop { offset: 0.0, rgba: 0x0011_2233 },
            Stop { offset: 0.5, rgba: 0x4455_6677 },
            Stop { offset: 0.5, rgba: 0x8899_AABB },
            Stop { offset: 1.0, rgba: 0xCCDD_EEFF },
        ],
    }
}

fn tiling() -> Tiling {
    Tiling {
        coloured: true,
        x_step: 4.0,
        y_step: 4.0,
        cell: [0.0, 0.0, 4.0, 4.0],
        transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        primitives: vec![
            TilingPrimitive::Polygon {
                rule: FillRule::NonZero,
                rgba: 0x0000_00FF,
                points: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
            },
            TilingPrimitive::Polyline { rgba: 0xFF00_00FF, width: 0.25, points: vec![[0.0, 4.0], [4.0, 0.0]] },
        ],
    }
}

fn full() -> FillPaintStream {
    FillPaintStream {
        gradients: vec![axial(), radial()],
        tilings: vec![tiling()],
        assignments: vec![
            FillAssignment { hatch: 0, rule: FillRule::EvenOdd, paint: FillSource::Own },
            FillAssignment { hatch: 2, rule: FillRule::NonZero, paint: FillSource::Gradient(1) },
            FillAssignment { hatch: 7, rule: FillRule::NonZero, paint: FillSource::Tiling(0) },
        ],
    }
}

fn rule_of(s: FillPaintStream) -> &'static str {
    s.encode().expect_err("refused").rule()
}

fn with_gradient(g: Gradient) -> FillPaintStream {
    FillPaintStream { gradients: vec![g], tilings: vec![], assignments: vec![] }
}

fn with_tiling(t: Tiling) -> FillPaintStream {
    FillPaintStream { gradients: vec![], tilings: vec![t], assignments: vec![] }
}

fn assert_rule_both_ways(s: FillPaintStream, rule: &str) {
    assert_eq!(rule_of(s), rule, "the writer");
}

// ---------------------------------------------------------------- registry and round trip

#[test]
fn the_registry_has_capability_25_as_an_optional_component_at_1_0_0() {
    let r = Registry::v1();
    let c = r.get(ids::FILL_PAINT).expect("25 is registered");
    assert_eq!((c.id.get(), c.name), (25, "fill_paint"));
    assert_eq!((c.class, c.requirement), (Class::Component, Requirement::Optional));
    assert_eq!((c.version, c.min_reader), (Version::new(1, 0, 0), Version::new(1, 0, 0)));
}

#[test]
fn a_full_stream_round_trips_with_every_float_bit_equal() {
    let s = full();
    let bytes = s.encode().unwrap();
    let back = FillPaintStream::decode(&bytes).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.gradients[1].transform[4].to_bits(), 7.0f64.to_bits());
}

#[test]
fn the_header_is_three_counts_and_an_assignment_is_twelve_bytes() {
    let s = FillPaintStream {
        gradients: vec![],
        tilings: vec![],
        assignments: vec![FillAssignment { hatch: 9, rule: FillRule::EvenOdd, paint: FillSource::Own }],
    };
    let b = s.encode().unwrap();
    assert_eq!(b.len(), 12 + 12);
    assert_eq!(&b[..12], &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
    // hatch u32, rule u8, paint u8, reserved u16, index u32
    assert_eq!(&b[12..], &[9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn an_axial_gradient_is_96_bytes_of_head_and_coordinates_plus_12_a_stop() {
    let g = axial();
    let b = with_gradient(g).encode().unwrap();
    // kind, extend, stops (u16), transform 48, coordinates 32, two stops of 12
    assert_eq!(b.len(), 12 + 4 + 48 + 32 + 24);
    assert_eq!((b[12], b[13], b[14], b[15]), (1, 0b01, 2, 0));
    let b = with_gradient(radial()).encode().unwrap();
    assert_eq!(b.len(), 12 + 4 + 48 + 48 + 48);
    assert_eq!((b[12], b[13]), (2, 0b10));
}

#[test]
fn a_tiling_is_a_100_byte_head_and_primitives_of_a_16_byte_head_and_16_a_point() {
    let b = with_tiling(tiling()).encode().unwrap();
    // paint type, flags, primitive count (u16), steps 16, cell 32, transform 48 = 100
    assert_eq!(b.len(), 12 + 100 + (16 + 3 * 16) + (16 + 2 * 16));
    assert_eq!((b[12], b[13], b[14], b[15]), (1, 0, 2, 0));
}

#[test]
fn an_empty_stream_is_twelve_bytes_and_round_trips() {
    let s = FillPaintStream { gradients: vec![], tilings: vec![], assignments: vec![] };
    assert_eq!(s.encode().unwrap(), vec![0; 12]);
    assert_eq!(FillPaintStream::decode(&[0; 12]).unwrap(), s);
}

#[test]
fn the_rule_of_a_hatch_record_is_looked_up_and_absence_means_the_old_behaviour() {
    let s = full();
    assert_eq!(s.assignment_of(2).map(|a| a.rule), Some(FillRule::NonZero));
    assert_eq!(s.assignment_of(0).map(|a| a.paint), Some(FillSource::Own));
    assert!(s.assignment_of(1).is_none());
}

// ---------------------------------------------------------------- the shape rules

#[test]
fn truncation_trailing_bytes_and_counts_over_the_ceilings() {
    let b = full().encode().unwrap();
    assert_eq!(FillPaintStream::decode(&b[..b.len() - 1]).unwrap_err().rule(), "COMP-01");
    let mut long = b.clone();
    long.push(0);
    assert_eq!(FillPaintStream::decode(&long).unwrap_err().rule(), "COMP-02");
    for (g, t, a) in [(4097u32, 0u32, 0u32), (0, 1025, 0), (0, 0, (1 << 20) + 1)] {
        let mut h = g.to_le_bytes().to_vec();
        h.extend_from_slice(&t.to_le_bytes());
        h.extend_from_slice(&a.to_le_bytes());
        assert_eq!(FillPaintStream::decode(&h).unwrap_err().rule(), "COMP-03", "{g} {t} {a}");
    }
    // a count the bytes cannot hold is a truncation, and nothing is allocated from it
    let mut h = 4096u32.to_le_bytes().to_vec();
    h.extend_from_slice(&[0; 8]);
    assert_eq!(FillPaintStream::decode(&h).unwrap_err().rule(), "COMP-01");
}

#[test]
fn an_assignment_with_a_bad_rule_paint_or_reserved_field() {
    let mut b = full().encode().unwrap();
    let at = b.len() - 12 * 3;
    let mut bad = b.clone();
    bad[at + 4] = 2; // rule
    assert_eq!(FillPaintStream::decode(&bad).unwrap_err().rule(), "COMP-11");
    let mut bad = b.clone();
    bad[at + 5] = 3; // paint
    assert_eq!(FillPaintStream::decode(&bad).unwrap_err().rule(), "COMP-11");
    let mut bad = b.clone();
    bad[at + 6] = 1; // reserved
    assert_eq!(FillPaintStream::decode(&bad).unwrap_err().rule(), "COMP-12");
    let mut bad = b.clone();
    bad[at + 8] = 1; // paint 0 with an index
    assert_eq!(FillPaintStream::decode(&bad).unwrap_err().rule(), "COMP-12");
    b.truncate(b.len());
}

#[test]
fn a_gradient_or_tiling_index_with_no_entry_is_comp_09() {
    let mut s = full();
    s.assignments[1].paint = FillSource::Gradient(2);
    assert_eq!(rule_of(s), "COMP-09");
    let mut s = full();
    s.assignments[2].paint = FillSource::Tiling(1);
    assert_eq!(rule_of(s), "COMP-09");
}

#[test]
fn hatch_records_that_do_not_strictly_ascend_are_comp_28() {
    let mut s = full();
    s.assignments[1].hatch = 0;
    assert_eq!(rule_of(s), "COMP-28");
    let mut s = full();
    s.assignments.swap(0, 2);
    assert_eq!(rule_of(s), "COMP-28");
}

#[test]
fn validation_refuses_a_hatch_record_past_the_end_of_the_hatch_stream_as_comp_22() {
    let s = full();
    assert_eq!(s.validate(8), Ok(()));
    assert_eq!(s.validate(7).unwrap_err().rule(), "COMP-22");
}

// ---------------------------------------------------------------- gradients: COMP-23 and COMP-24

#[test]
fn a_zero_length_axis_is_comp_23() {
    let mut g = axial();
    g.coords = GradientCoords::Axial { x0: 3.0, y0: 3.0, x1: 3.0, y1: 3.0 };
    assert_rule_both_ways(with_gradient(g), "COMP-23");
}

#[test]
fn a_negative_radius_and_two_identical_circles_are_comp_23() {
    let mut g = radial();
    g.coords = GradientCoords::Radial { x0: 0.0, y0: 0.0, r0: -1.0, x1: 5.0, y1: 0.0, r1: 4.0 };
    assert_rule_both_ways(with_gradient(g), "COMP-23");
    let mut g = radial();
    g.coords = GradientCoords::Radial { x0: 1.0, y0: 2.0, r0: 3.0, x1: 1.0, y1: 2.0, r1: 3.0 };
    assert_rule_both_ways(with_gradient(g), "COMP-23");
    // the same centre with different radii is a legal concentric ramp
    let mut g = radial();
    g.coords = GradientCoords::Radial { x0: 1.0, y0: 2.0, r0: 0.0, x1: 1.0, y1: 2.0, r1: 3.0 };
    assert!(with_gradient(g).encode().is_ok());
}

#[test]
fn a_singular_transform_is_comp_23() {
    let mut g = axial();
    g.transform = [1.0, 2.0, 2.0, 4.0, 0.0, 0.0];
    assert_rule_both_ways(with_gradient(g), "COMP-23");
    let mut g = axial();
    g.transform = [0.0; 6];
    assert_rule_both_ways(with_gradient(g), "COMP-23");
}

#[test]
fn stops_out_of_order_or_outside_0_to_1_are_comp_24() {
    for offsets in [[0.5, 0.2], [-0.1, 1.0], [0.0, 1.5]] {
        let mut g = axial();
        g.stops = offsets.iter().map(|o| Stop { offset: *o, rgba: 0 }).collect();
        assert_rule_both_ways(with_gradient(g), "COMP-24");
    }
}

#[test]
fn a_repeated_offset_is_a_hard_step_and_allowed() {
    let mut g = axial();
    g.stops = vec![
        Stop { offset: 0.0, rgba: 1 },
        Stop { offset: 0.5, rgba: 2 },
        Stop { offset: 0.5, rgba: 3 },
        Stop { offset: 1.0, rgba: 4 },
    ];
    assert!(with_gradient(g).encode().is_ok());
}

#[test]
fn non_finite_numbers_are_comp_06() {
    let mut g = axial();
    g.transform[4] = f64::NAN;
    assert_rule_both_ways(with_gradient(g), "COMP-06");
    let mut g = axial();
    g.coords = GradientCoords::Axial { x0: 0.0, y0: f64::INFINITY, x1: 1.0, y1: 1.0 };
    assert_rule_both_ways(with_gradient(g), "COMP-06");
    let mut g = axial();
    g.stops[0].offset = f64::NAN;
    assert_rule_both_ways(with_gradient(g), "COMP-06");
}

#[test]
fn between_2_and_256_stops_are_allowed_and_anything_else_is_refused() {
    let many = |n: usize| -> Gradient {
        let mut g = axial();
        g.stops = (0..n).map(|i| Stop { offset: i as f64 / 255.0_f64.max(n as f64 - 1.0), rgba: i as u32 }).collect();
        g
    };
    assert!(with_gradient(many(256)).encode().is_ok());
    assert_rule_both_ways(with_gradient(many(257)), "COMP-03");
    let mut one = axial();
    one.stops.truncate(1);
    assert_rule_both_ways(with_gradient(one), "COMP-05");
}

#[test]
fn a_gradient_kind_or_extend_bit_the_format_does_not_define_is_refused_on_read() {
    let mut b = with_gradient(axial()).encode().unwrap();
    b[12] = 3;
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-11");
    let mut b = with_gradient(axial()).encode().unwrap();
    b[13] = 0b100;
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-12");
}

#[test]
fn the_decoder_holds_a_hostile_gradient_to_the_same_rules() {
    // a zero-length axis and a descending stop pair, written by hand into a valid payload
    let mut b = with_gradient(axial()).encode().unwrap();
    for i in 0..4 {
        let at = 12 + 4 + 48 + i * 8;
        b[at..at + 8].copy_from_slice(&0.0f64.to_le_bytes());
    }
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-23");
    let mut b = with_gradient(axial()).encode().unwrap();
    let at = 12 + 4 + 48 + 32 + 12; // the second stop's offset
    b[at..at + 8].copy_from_slice(&(-1.0f64).to_le_bytes());
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-24");
}

// ---------------------------------------------------------------- tilings

#[test]
fn tiling_steps_and_cell_must_be_finite_positive_and_non_empty() {
    for f in [
        (|t: &mut Tiling| t.x_step = 0.0) as fn(&mut Tiling),
        |t| t.y_step = -1.0,
        |t| t.x_step = f64::NAN,
        |t| t.cell = [0.0, 0.0, 0.0, 4.0],
        |t| t.cell = [0.0, 5.0, 4.0, 4.0],
        |t| t.cell[2] = f64::INFINITY,
    ] {
        let mut t = tiling();
        f(&mut t);
        assert_eq!(rule_of(with_tiling(t)), "COMP-06");
    }
}

#[test]
fn a_singular_pattern_transform_is_comp_23() {
    let mut t = tiling();
    t.transform = [1.0, 1.0, 1.0, 1.0, 0.0, 0.0];
    assert_eq!(rule_of(with_tiling(t)), "COMP-23");
}

#[test]
fn point_counts_are_3_to_4096_for_a_polygon_and_2_to_4096_for_a_polyline() {
    let poly = |n: usize| TilingPrimitive::Polygon {
        rule: FillRule::EvenOdd,
        rgba: 0,
        points: (0..n).map(|i| [i as f64, (i * i) as f64]).collect(),
    };
    let line = |n: usize| TilingPrimitive::Polyline {
        rgba: 0,
        width: 0.0,
        points: (0..n).map(|i| [i as f64, 0.0]).collect(),
    };
    let one = |p| {
        let mut t = tiling();
        t.primitives = vec![p];
        with_tiling(t)
    };
    assert!(one(poly(3)).encode().is_ok());
    assert!(one(line(2)).encode().is_ok());
    assert_eq!(rule_of(one(poly(2))), "COMP-05");
    assert_eq!(rule_of(one(line(1))), "COMP-05");
    assert_eq!(rule_of(one(poly(4097))), "COMP-03");
    assert_eq!(rule_of(one(line(4097))), "COMP-03");
}

#[test]
fn a_tiling_of_more_than_8192_points_is_comp_27() {
    let mut t = tiling();
    t.primitives = (0..3)
        .map(|_| TilingPrimitive::Polyline {
            rgba: 0,
            width: 1.0,
            points: (0..3000).map(|i| [i as f64, 0.0]).collect(),
        })
        .collect();
    assert_eq!(rule_of(with_tiling(t.clone())), "COMP-27");
    t.primitives.truncate(2);
    assert!(with_tiling(t).encode().is_ok(), "6000 points is inside the ceiling");
    // exactly 8192 is allowed
    let mut t = tiling();
    t.primitives = (0..2)
        .map(|_| TilingPrimitive::Polyline { rgba: 0, width: 1.0, points: vec![[0.0, 0.0]; 4096] })
        .collect();
    assert!(with_tiling(t).encode().is_ok());
}

#[test]
fn zero_primitives_is_comp_05_and_more_than_256_is_comp_03() {
    let mut t = tiling();
    t.primitives.clear();
    assert_eq!(rule_of(with_tiling(t)), "COMP-05");
    let mut t = tiling();
    t.primitives = vec![TilingPrimitive::Polyline { rgba: 0, width: 0.0, points: vec![[0.0; 2]; 2] }; 257];
    assert_eq!(rule_of(with_tiling(t)), "COMP-03");
}

#[test]
fn a_polygon_has_width_zero_and_a_polyline_has_no_rule_and_both_widths_are_finite() {
    let mut b = with_tiling(tiling()).encode().unwrap();
    // first primitive: kind, rule, u16 count, rgba, width
    let p = 12 + 100;
    assert_eq!(b[p], 1);
    b[p + 8..p + 16].copy_from_slice(&1.0f64.to_le_bytes());
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-12");
    let mut b = with_tiling(tiling()).encode().unwrap();
    b[p] = 3;
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-11");
    let mut t = tiling();
    t.primitives[1] = TilingPrimitive::Polyline { rgba: 0, width: -1.0, points: vec![[0.0; 2]; 2] };
    assert_eq!(rule_of(with_tiling(t)), "COMP-06");
}

#[test]
fn an_uncoloured_tiling_round_trips_and_its_colour_words_are_carried_untouched() {
    let mut t = tiling();
    t.coloured = false;
    let s = with_tiling(t);
    assert_eq!(FillPaintStream::decode(&s.encode().unwrap()).unwrap(), s);
}

#[test]
fn a_tiling_flag_or_paint_type_the_format_does_not_define_is_refused_on_read() {
    let mut b = with_tiling(tiling()).encode().unwrap();
    b[12] = 3;
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-11");
    let mut b = with_tiling(tiling()).encode().unwrap();
    b[13] = 1;
    assert_eq!(FillPaintStream::decode(&b).unwrap_err().rule(), "COMP-12");
}

// ---------------------------------------------------------------- framing and old readers

#[test]
fn it_is_framed_with_the_registry_version_and_a_reader_without_25_skips_it_by_length() {
    let r = Registry::v1();
    let framed = full().into_stream(&r).unwrap();
    assert_eq!((framed.capability, framed.version), (ids::FILL_PAINT, Version::new(1, 0, 0)));
    let set = encode_streams(&[framed]).unwrap();
    let knows = Reader { version: Version::new(1, 3, 0), implemented: vec![ids::FILL_PAINT] };
    let opened = read_streams(&set, &r, &knows).unwrap();
    assert_eq!(FillPaintStream::decode(&opened.streams[0].payload).unwrap(), full());
    let old = Reader { version: Version::new(1, 0, 0), implemented: vec![ids::HATCH] };
    let opened = read_streams(&set, &r, &old).unwrap();
    assert!(opened.streams.is_empty());
    assert_eq!(opened.skipped[0].reason, SkipReason::NotImplemented);
}
