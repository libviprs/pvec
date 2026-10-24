//! The writer's contract (issue P4.4): what is in the file, in what order, and what it refuses.

mod support;

use pvec_capabilities::{CapabilityId, Class, Registry, Requirement, Version};
use pvec_components::Stream;
use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::blocks::arena::ArenaError;
use pvec_container::blocks::hot::HotError;
use pvec_container::directory::{BlockKind, FLAG_HAS_BOUNDS, FLAG_MUST_UNDERSTAND, parse_directory};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriteError};
use pvec_container::{Bounds, ContainerError, Header, Manifest};
use pvec_geometry::{
    BSpline, Bezier, Circle, Curve, Ellipse, Line, Placement2, Point2, Polyline, RationalBSpline, Trimmed,
    Vector2,
};
use support::*;

fn open(bytes: &[u8]) -> (Header, Manifest, Vec<pvec_container::directory::BlockEntry>) {
    let len = bytes.len() as u64;
    let header = Header::parse(&bytes[..bytes.len().min(4096)], len).unwrap();
    let (m0, m1) =
        (header.manifest_offset as usize, (header.manifest_offset + header.manifest_length) as usize);
    let manifest = Manifest::parse(&bytes[m0..m1], &header, len).unwrap();
    let d = manifest.directories[0];
    let entries = parse_directory(
        &bytes[d.offset as usize..(d.offset + d.length) as usize],
        header.header_len as u64,
        len,
    )
    .unwrap();
    (header, manifest, entries)
}

fn meta(id: u32, cell: u32, b: Bounds) -> EntityMeta {
    EntityMeta { id: eid(id), style: 0, cell, bounds: b }
}

fn unit_box() -> Bounds {
    Bounds { min: [0.0, 0.0], max: [1.0, 1.0] }
}

fn writer(input: &Input) -> ContainerWriter {
    ContainerWriter::new(input.config.clone())
}

/// The first error from pushing and finishing, wherever the writer chooses to report it.
fn first_error(
    mut w: ContainerWriter,
    pushes: impl FnOnce(&mut ContainerWriter) -> Result<(), WriteError>,
) -> WriteError {
    if let Err(e) = pushes(&mut w) {
        return e;
    }
    w.finish().expect_err("the writer accepted what it should refuse")
}

#[test]
fn the_file_is_header_manifest_blocks_and_directory_with_nothing_else() {
    for fx in fixtures() {
        let bytes = fx.write().unwrap();
        let (header, manifest, entries) = open(&bytes);
        assert_eq!(header.header_len as usize, Header::LEN, "{}", fx.name);
        assert_eq!(header.manifest_offset, 64, "{}", fx.name);
        let after_manifest = header.manifest_offset + header.manifest_length;
        assert!(entries[0].offset >= after_manifest && entries[0].offset - after_manifest < 8, "{}", fx.name);
        let dir = manifest.directories[0];
        assert_eq!(dir.offset + dir.length, bytes.len() as u64, "{}: bytes after the directory", fx.name);
        let last = entries.last().unwrap();
        let end = last.offset + u64::from(last.stored_length);
        assert!(dir.offset >= end && dir.offset - end < 8, "{}: a gap before the directory", fx.name);
        assert_eq!(dir.length as usize, entries.len() * 64);
        for w in entries.windows(2) {
            let end = w[0].offset + u64::from(w[0].stored_length);
            assert!(w[1].offset >= end && w[1].offset - end < 8, "{}: a gap between blocks", fx.name);
        }
        assert_eq!(header.directory_table_offset, 64 + 128 + 16 * manifest.capabilities.len() as u64);
    }
}

#[test]
fn every_scalar_is_little_endian_so_the_magic_and_marker_read_the_same_everywhere() {
    let bytes = minimal().write().unwrap();
    assert_eq!(&bytes[..8], &[0x89, 0x50, 0x56, 0x45, 0x43, 0x0d, 0x0a, 0x1a]);
    assert_eq!(&bytes[12..16], &[0x04, 0x03, 0x02, 0x01]);
    assert_eq!(&bytes[16..18], &[64, 0]);
    // the one entity is the line (0, 0) to (3, 4): its f64 columns are stored little-endian
    let (_, _, entries) = open(&bytes);
    let block = &bytes[entries[0].offset as usize..][..entries[0].stored_length as usize];
    let columns = &block[block.len() - 32..];
    let want: Vec<u8> = [0.0f64, 0.0, 3.0, 4.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    assert_eq!(columns, &want[..]);
}

#[test]
fn capabilities_come_from_the_registry_in_id_order() {
    let fx = mixed();
    let bytes = fx.write().unwrap();
    let (_, manifest, _) = open(&bytes);
    let mut want: Vec<u32> = fx.config.capabilities.iter().map(|c| c.get()).collect();
    want.extend(fx.streams().map(|s| s.capability.get()));
    want.sort_unstable();
    want.dedup();
    let got: Vec<u32> = manifest.capabilities.iter().map(|d| d.id.get()).collect();
    assert_eq!(got, want);
    let registry = Registry::v1();
    for d in &manifest.capabilities {
        let c = registry.get(d.id).unwrap();
        assert_eq!(d.version, c.version);
        assert_eq!(d.requirement, c.requirement);
        if c.class == Class::Component {
            assert_eq!(d.requirement, Requirement::Optional);
        }
    }
    assert!(manifest.min_reader >= Version::new(1, 0, 0));
}

#[test]
fn a_stream_declares_its_capability_even_when_the_caller_forgot_to() {
    let mut fx = minimal();
    fx.items.push(Item::Stream(Stream {
        capability: CapabilityId::new(14),
        version: Version::new(1, 0, 0),
        payload: vec![1, 2, 3],
    }));
    let (_, manifest, _) = open(&fx.write().unwrap());
    assert!(manifest.capabilities.iter().any(|d| d.id.get() == 14));
}

#[test]
fn a_capability_the_registry_does_not_know_is_refused() {
    let mut fx = minimal();
    fx.config.capabilities.push(CapabilityId::new(9999));
    assert_eq!(fx.write().unwrap_err(), WriteError::UnknownCapability { id: 9999 });
}

#[test]
fn the_producer_identity_is_the_callers_and_an_empty_one_is_refused() {
    let fx = minimal();
    let (_, manifest, _) = open(&fx.write().unwrap());
    assert_eq!(manifest.metadata, fx.config.metadata);
    let mut empty = minimal();
    empty.config.metadata.producer_id = String::new();
    assert_eq!(empty.write().unwrap_err(), WriteError::Container(ContainerError::MissingProducer));
}

#[test]
fn source_bounds_default_to_the_union_of_the_entities_and_an_explicit_value_wins() {
    let fx = plan();
    let (_, manifest, _) = open(&fx.write().unwrap());
    let want = fx.entities().map(Ent::bounds).reduce(union).unwrap();
    assert_eq!(manifest.bounds, want);
    let explicit = mixed();
    let (_, manifest, _) = open(&explicit.write().unwrap());
    assert_eq!(manifest.bounds, explicit.config.bounds.unwrap());
}

#[test]
fn hot_blocks_are_one_per_cell_and_geometry_type_in_cell_then_type_order() {
    let fx = plan();
    let (_, _, entries) = open(&fx.write().unwrap());
    let hot: Vec<_> = entries.iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)).collect();
    assert_eq!(hot.len(), 7 * 4, "seven cells of four geometry types");
    assert_eq!(hot.iter().map(|e| e.count as usize).sum::<usize>(), 480);
    let keys: Vec<(u32, u16)> = hot.iter().map(|e| (e.cell, e.subtype)).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(keys, sorted);
    for e in &hot {
        assert_eq!(e.flags & FLAG_MUST_UNDERSTAND, FLAG_MUST_UNDERSTAND);
        assert_eq!(e.flags & FLAG_HAS_BOUNDS, FLAG_HAS_BOUNDS);
        assert!(e.bounds.is_some());
    }
}

#[test]
fn a_block_entry_carries_the_union_of_its_entities_bounds() {
    let mut w = ContainerWriter::new(minimal().config);
    let (b1, b2) =
        (Bounds { min: [0.0, 1.0], max: [2.0, 3.0] }, Bounds { min: [-1.0, 0.5], max: [1.0, 9.0] });
    w.push_line(meta(1, 4, b1), [0.0, 1.0], [2.0, 3.0]).unwrap();
    w.push_line(meta(0, 4, b2), [-1.0, 0.5], [1.0, 9.0]).unwrap();
    let (_, _, entries) = open(&w.finish().unwrap());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].cell, 4);
    assert_eq!(entries[0].count, 2);
    assert_eq!(entries[0].bounds, Some(Bounds { min: [-1.0, 0.5], max: [2.0, 9.0] }));
}

#[test]
fn advanced_curves_are_a_record_block_and_five_arenas_in_cell_zero() {
    let fx = spline_heavy();
    let (_, _, entries) = open(&fx.write().unwrap());
    let records: Vec<_> = entries.iter().filter(|e| e.block_kind() == Some(BlockKind::Records)).collect();
    let arenas: Vec<_> = entries.iter().filter(|e| e.block_kind() == Some(BlockKind::Arena)).collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].count, 370, "300 splines and 70 composites");
    assert_eq!(arenas.iter().map(|e| e.subtype).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
    for e in records.iter().chain(&arenas) {
        assert_eq!(e.cell, 0);
        assert_eq!(e.flags & FLAG_MUST_UNDERSTAND, FLAG_MUST_UNDERSTAND);
    }
    assert!(records[0].bounds.is_some());
    assert!(arenas.iter().all(|e| e.bounds.is_none() && e.flags & FLAG_HAS_BOUNDS == 0));
}

#[test]
fn component_streams_are_one_optional_block_in_capability_order() {
    let fx = mixed();
    let (_, _, entries) = open(&fx.write().unwrap());
    let comp: Vec<_> = entries.iter().filter(|e| e.block_kind() == Some(BlockKind::Components)).collect();
    assert_eq!(comp.len(), 1);
    assert_eq!(comp[0].count, 5);
    assert_eq!(
        comp[0].flags & FLAG_MUST_UNDERSTAND,
        0,
        "a component set is skipped by a reader that does not know it"
    );
    assert_eq!(entries.last().unwrap().kind, BlockKind::Components.id(), "kind order puts components last");
}

#[test]
fn nothing_to_write_is_refused() {
    assert_eq!(writer(&minimal()).finish().unwrap_err(), WriteError::Empty);
    let mut w = writer(&minimal());
    w.push_stream(Stream {
        capability: CapabilityId::new(14),
        version: Version::new(1, 0, 0),
        payload: vec![0],
    })
    .unwrap();
    assert_eq!(w.finish().unwrap_err(), WriteError::Empty, "streams alone are not an artifact");
}

#[test]
fn an_id_used_by_two_entities_is_refused_even_across_geometry_types() {
    let e = first_error(writer(&minimal()), |w| {
        w.push_line(meta(7, 0, unit_box()), [0.0, 0.0], [1.0, 1.0])?;
        w.push_polyline(meta(7, 0, unit_box()), &[[0.0, 0.0], [1.0, 1.0]])
    });
    assert_eq!(e, WriteError::DuplicateId { id: 7 });
    let e = first_error(writer(&minimal()), |w| {
        w.push_line(meta(3, 0, unit_box()), [0.0, 0.0], [1.0, 1.0])?;
        let k = clamped(1, 2);
        w.push_spline(
            meta(3, 0, unit_box()),
            SplineSpec {
                degree: 1,
                knots: &k,
                x: &[0.0, 1.0],
                y: &[0.0, 1.0],
                weights: None,
                closed: false,
                periodic: false,
            },
        )
    });
    assert_eq!(e, WriteError::DuplicateId { id: 3 });
}

fn clamped(degree: usize, n: usize) -> Vec<f64> {
    let mut k = vec![0.0; degree + 1];
    k.extend((1..=n - degree - 1).map(|v| v as f64));
    k.extend(std::iter::repeat_n((n - degree) as f64, degree + 1));
    k
}

#[test]
fn two_streams_of_one_capability_are_refused() {
    let s = |p: u8| Stream {
        capability: CapabilityId::new(15),
        version: Version::new(1, 0, 0),
        payload: vec![p],
    };
    let e = first_error(writer(&minimal()), |w| {
        w.push_line(meta(0, 0, unit_box()), [0.0, 0.0], [1.0, 1.0])?;
        w.push_stream(s(1))?;
        w.push_stream(s(2))
    });
    assert_eq!(e, WriteError::DuplicateStream { capability: 15 });
}

#[test]
fn bounds_that_are_not_finite_or_not_ordered_are_refused() {
    for b in [
        Bounds { min: [f64::NAN, 0.0], max: [1.0, 1.0] },
        Bounds { min: [0.0, 0.0], max: [f64::INFINITY, 1.0] },
        Bounds { min: [2.0, 0.0], max: [1.0, 1.0] },
    ] {
        let e = first_error(writer(&minimal()), |w| w.push_line(meta(5, 0, b), [0.0, 0.0], [1.0, 1.0]));
        assert_eq!(e, WriteError::BadBounds { id: Some(5) });
    }
    let mut fx = minimal();
    fx.config.bounds = Some(Bounds { min: [3.0, 0.0], max: [1.0, 1.0] });
    assert_eq!(fx.write().unwrap_err(), WriteError::BadBounds { id: None });
}

#[test]
fn an_advanced_curve_with_a_cell_is_refused_because_cells_belong_to_hot_blocks() {
    let k = clamped(1, 2);
    let e = first_error(writer(&minimal()), |w| {
        w.push_spline(
            meta(1, 3, unit_box()),
            SplineSpec {
                degree: 1,
                knots: &k,
                x: &[0.0, 1.0],
                y: &[0.0, 1.0],
                weights: None,
                closed: false,
                periodic: false,
            },
        )
    });
    assert_eq!(e, WriteError::CellOnAdvanced { id: 1 });
}

#[test]
fn a_block_builder_refusal_comes_back_typed() {
    let e =
        first_error(writer(&minimal()), |w| w.push_line(meta(0, 0, unit_box()), [f64::NAN, 0.0], [1.0, 1.0]));
    assert!(matches!(e, WriteError::Hot(HotError::NonFinite { .. })), "{e:?}");
    let e = first_error(writer(&minimal()), |w| {
        w.push_spline(
            meta(0, 0, unit_box()),
            SplineSpec {
                degree: 2,
                knots: &[0.0, 1.0],
                x: &[0.0, 1.0],
                y: &[0.0, 1.0],
                weights: None,
                closed: false,
                periodic: false,
            },
        )
    });
    assert!(matches!(e, WriteError::Arena(ArenaError::KnotCount { .. })), "{e:?}");
    let e = first_error(writer(&minimal()), |w| {
        w.push_composite(meta(0, 0, unit_box()), &[eid(99)], None, false)
    });
    assert_eq!(e, WriteError::Arena(ArenaError::UnknownSegment { id: 99 }));
}

// ---- curves of the semantic model ----

fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn as_bounds(c: &Curve) -> Bounds {
    use pvec_geometry::Curve2d;
    let b = c.bounds().unwrap();
    Bounds { min: [b.min().x, b.min().y], max: [b.max().x, b.max().y] }
}

fn via_curve(c: &Curve) -> Vec<u8> {
    let mut w = writer(&minimal());
    w.push_curve(eid(0), 2, 0, c).unwrap();
    w.finish().unwrap()
}

#[test]
fn a_polyline_of_the_model_is_the_same_bytes_as_the_hot_polyline() {
    let c = Curve::Polyline(Polyline::new(vec![pt(0.0, 0.0), pt(1.5, 2.0), pt(3.0, 0.5)]).unwrap());
    let mut w = writer(&minimal());
    w.push_polyline(
        EntityMeta { id: eid(0), style: 2, cell: 0, bounds: as_bounds(&c) },
        &[[0.0, 0.0], [1.5, 2.0], [3.0, 0.5]],
    )
    .unwrap();
    assert_eq!(via_curve(&c), w.finish().unwrap());
}

#[test]
fn a_bspline_and_a_rational_bspline_of_the_model_are_the_same_bytes_as_a_spline_record() {
    let cps = vec![pt(0.0, 0.0), pt(1.0, 2.0), pt(2.0, -1.0), pt(4.0, 0.5)];
    let knots = clamped(2, 4);
    let plain = Curve::BSpline(BSpline::new(2, cps.clone(), knots.clone()).unwrap());
    let rational = Curve::RationalBSpline(
        RationalBSpline::new(2, cps, knots.clone(), vec![1.0, 2.0, 1.5, 1.0]).unwrap(),
    );
    let (x, y) = ([0.0, 1.0, 2.0, 4.0], [0.0, 2.0, -1.0, 0.5]);
    for (curve, weights) in [(&plain, None), (&rational, Some(&[1.0, 2.0, 1.5, 1.0][..]))] {
        let mut w = writer(&minimal());
        w.push_spline(
            EntityMeta { id: eid(0), style: 2, cell: 0, bounds: as_bounds(curve) },
            SplineSpec { degree: 2, knots: &knots, x: &x, y: &y, weights, closed: false, periodic: false },
        )
        .unwrap();
        assert_eq!(via_curve(curve), w.finish().unwrap());
    }
}

#[test]
fn a_bezier_is_a_clamped_spline_of_the_same_degree() {
    let b = Bezier::new(vec![pt(0.0, 0.0), pt(1.0, 3.0), pt(3.0, 3.0), pt(4.0, 0.0)]).unwrap();
    let curve = Curve::Bezier(b);
    let mut w = writer(&minimal());
    w.push_spline(
        EntityMeta { id: eid(0), style: 2, cell: 0, bounds: as_bounds(&curve) },
        SplineSpec {
            degree: 3,
            knots: &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            x: &[0.0, 1.0, 3.0, 4.0],
            y: &[0.0, 3.0, 3.0, 0.0],
            weights: None,
            closed: false,
            periodic: false,
        },
    )
    .unwrap();
    assert_eq!(via_curve(&curve), w.finish().unwrap());
}

#[test]
fn a_trimmed_line_is_the_hot_line_between_its_end_points() {
    let line = Line::new(pt(1.0, 1.0), Vector2::new(1.0, 0.0)).unwrap();
    let curve = Curve::Trimmed(Trimmed::new(Curve::Line(line), 0.0, 4.0, true).unwrap());
    let mut w = writer(&minimal());
    w.push_line(
        EntityMeta { id: eid(0), style: 2, cell: 0, bounds: as_bounds(&curve) },
        [1.0, 1.0],
        [5.0, 1.0],
    )
    .unwrap();
    assert_eq!(via_curve(&curve), w.finish().unwrap());
}

#[test]
fn a_circle_and_a_trimmed_circle_with_an_axis_aligned_frame_are_arcs() {
    let frame = Placement2::new(pt(2.0, 3.0), Vector2::new(1.0, 0.0)).unwrap();
    let circle = Circle::new(frame, 5.0).unwrap();
    let full = Curve::Circle(circle);
    let mut w = writer(&minimal());
    w.push_circle_arc(
        EntityMeta { id: eid(0), style: 2, cell: 0, bounds: as_bounds(&full) },
        [2.0, 3.0],
        5.0,
        [0.0, std::f64::consts::TAU],
    )
    .unwrap();
    assert_eq!(via_curve(&full), w.finish().unwrap());

    let arc = Curve::Trimmed(Trimmed::new(Curve::Circle(circle), 0.5, 2.0, true).unwrap());
    let mut w = writer(&minimal());
    w.push_circle_arc(
        EntityMeta { id: eid(0), style: 2, cell: 0, bounds: as_bounds(&arc) },
        [2.0, 3.0],
        5.0,
        [0.5, 1.5],
    )
    .unwrap();
    assert_eq!(via_curve(&arc), w.finish().unwrap());

    let ellipse = Ellipse::new(frame, 6.0, 2.0).unwrap();
    let e = Curve::Ellipse(ellipse);
    let mut w = writer(&minimal());
    w.push_ellipse_arc(
        EntityMeta { id: eid(0), style: 2, cell: 0, bounds: as_bounds(&e) },
        [2.0, 3.0],
        [6.0, 2.0],
        [0.0, 0.0, std::f64::consts::TAU],
    )
    .unwrap();
    assert_eq!(via_curve(&e), w.finish().unwrap());
}

#[test]
fn what_the_hot_forms_cannot_say_exactly_is_refused_and_not_approximated() {
    // a rotated frame needs atan2, whose last bit is a property of the C library and not of the format
    let rotated = Placement2::new(pt(0.0, 0.0), Vector2::new(1.0, 1.0)).unwrap();
    let c = Curve::Circle(Circle::new(rotated, 1.0).unwrap());
    let unbounded = Curve::Line(Line::new(pt(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap());
    let nested = Curve::Composite(
        pvec_geometry::Composite::new(vec![Curve::Polyline(
            Polyline::new(vec![pt(0.0, 0.0), pt(1.0, 0.0)]).unwrap(),
        )])
        .unwrap(),
    );
    for (curve, what) in
        [(c, "rotated placement"), (unbounded, "unbounded line"), (nested, "composite curve")]
    {
        let mut w = writer(&minimal());
        assert_eq!(w.push_curve(eid(0), 0, 0, &curve), Err(WriteError::Unsupported { what }));
    }
}
