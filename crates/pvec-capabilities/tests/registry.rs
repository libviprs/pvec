//! The registry as data (P3.4): stable ids, retired ids, the ISO coupling, class rules and the two
//! negotiation refusals the acceptance criteria name.
#![allow(missing_docs)]

use pvec_capabilities::*;

fn cap(id: u32, name: &'static str) -> Capability {
    Capability {
        id: CapabilityId::new(id),
        name,
        class: Class::Geometry,
        version: Version::new(1, 0, 0),
        iso_construct: "line",
        profile_section: 4,
        requirement: Requirement::Required,
        step: StepBehaviour::ExactForm,
        gis: GisBehaviour::NotApplicable,
        min_reader: Version::new(1, 0, 0),
        restriction: "test entry",
    }
}

fn component(id: u32, name: &'static str) -> Capability {
    Capability {
        class: Class::Component,
        requirement: Requirement::Optional,
        iso_construct: "",
        profile_section: 0,
        step: StepBehaviour::NotApplicable,
        ..cap(id, name)
    }
}

fn section(number: u32, constructs: &str) -> ProfileSection {
    ProfileSection { number, constructs: constructs.to_string() }
}

// ---------------------------------------------------------------- ids

#[test]
fn the_v1_registry_is_valid_and_its_ids_are_dense_unique_and_pinned() {
    let r = Registry::v1();
    let ids: Vec<u32> = r.entries().iter().map(|c| c.id.get()).collect();
    let n = ids.len() as u32;
    assert!(n >= 13, "the thirteen profile capabilities are registered");
    assert_eq!(ids, (1..=n).collect::<Vec<_>>(), "ids are dense and in order");
    let mut names: Vec<&str> = r.entries().iter().map(|c| c.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), r.entries().len(), "names are unique");
    // Pinned: a renumbering is a breaking change and has to turn this red.
    for (name, id) in [
        ("cartesian_point", 1),
        ("direction_vector", 2),
        ("axis_placement_2d", 3),
        ("line", 4),
        ("polyline", 5),
        ("circle", 6),
        ("ellipse", 7),
        ("trimmed_curve", 8),
        ("composite_curve", 9),
        ("bezier_curve", 10),
        ("b_spline_curve", 11),
        ("rational_b_spline_curve", 12),
        ("affine_transform_2d", 13),
    ] {
        assert_eq!(r.by_name(name).map(|c| c.id.get()), Some(id), "{name}");
    }
}

#[test]
fn the_id_constants_agree_with_the_registry() {
    let r = Registry::v1();
    for (id, name) in [
        (ids::CARTESIAN_POINT, "cartesian_point"),
        (ids::DIRECTION_VECTOR, "direction_vector"),
        (ids::AXIS_PLACEMENT_2D, "axis_placement_2d"),
        (ids::LINE, "line"),
        (ids::POLYLINE, "polyline"),
        (ids::CIRCLE, "circle"),
        (ids::ELLIPSE, "ellipse"),
        (ids::TRIMMED_CURVE, "trimmed_curve"),
        (ids::COMPOSITE_CURVE, "composite_curve"),
        (ids::BEZIER_CURVE, "bezier_curve"),
        (ids::B_SPLINE_CURVE, "b_spline_curve"),
        (ids::RATIONAL_B_SPLINE_CURVE, "rational_b_spline_curve"),
        (ids::AFFINE_TRANSFORM_2D, "affine_transform_2d"),
    ] {
        assert_eq!(r.get(id).map(|c| c.name), Some(name));
    }
}

#[test]
fn duplicate_ids_and_duplicate_names_are_refused() {
    let dup_id = Registry::new(vec![cap(1, "a"), cap(1, "b")], vec![]);
    assert_eq!(dup_id.unwrap_err(), RegistryError::DuplicateId(CapabilityId::new(1)));
    let dup_name = Registry::new(vec![cap(1, "a"), cap(2, "a")], vec![]);
    assert_eq!(dup_name.unwrap_err(), RegistryError::DuplicateName("a"));
    assert_eq!(Registry::new(vec![cap(0, "zero")], vec![]).unwrap_err(), RegistryError::ZeroId);
}

#[test]
fn a_retired_id_can_never_be_reused() {
    let retired = vec![Retired { id: CapabilityId::new(2), name: "old_thing", reason: "superseded by 3" }];
    let reuse = Registry::new(vec![cap(1, "a"), cap(2, "b"), cap(3, "c")], retired.clone());
    assert_eq!(reuse.unwrap_err(), RegistryError::ReusedRetiredId(CapabilityId::new(2)));
    // Retired and live are disjoint and together cover every id up to the highest.
    let ok = Registry::new(vec![cap(1, "a"), cap(3, "c")], retired).unwrap();
    assert_eq!(ok.retired().len(), 1);
    assert!(ok.get(CapabilityId::new(2)).is_none());
    assert!(ok.is_retired(CapabilityId::new(2)));
    assert!(!ok.is_retired(CapabilityId::new(3)));
}

#[test]
fn deleting_an_entry_without_retiring_it_leaves_an_unaccounted_id() {
    // id 2 is neither live nor retired: it was deleted, and its number could be handed out again.
    let r = Registry::new(vec![cap(1, "a"), cap(3, "c")], vec![]);
    assert_eq!(r.unwrap_err(), RegistryError::UnaccountedId(CapabilityId::new(2)));
}

#[test]
fn the_retired_list_itself_cannot_repeat_an_id() {
    let twice = vec![
        Retired { id: CapabilityId::new(1), name: "x", reason: "r" },
        Retired { id: CapabilityId::new(1), name: "y", reason: "r" },
    ];
    assert_eq!(
        Registry::new(vec![cap(2, "a")], twice).unwrap_err(),
        RegistryError::DuplicateRetired(CapabilityId::new(1))
    );
}

// ---------------------------------------------------------------- the ISO coupling

#[test]
fn every_geometry_entry_must_name_a_construct_with_a_section_in_the_profile() {
    let profile = vec![section(4, "`line`")];
    let r = Registry::new(vec![cap(1, "line")], vec![]).unwrap();
    assert!(r.validate_against_profile(&profile).is_ok());

    // no section with that number
    assert_eq!(
        r.validate_against_profile(&[section(5, "`polyline`")]).unwrap_err(),
        RegistryError::NoProfileSection { name: "line", section: 4 }
    );
    // the section exists but does not name the construct
    assert_eq!(
        r.validate_against_profile(&[section(4, "`polyline`")]).unwrap_err(),
        RegistryError::ConstructNotInSection { name: "line", construct: "line", section: 4 }
    );
    // an empty construct
    let mut e = cap(1, "line");
    e.iso_construct = "";
    let r = Registry::new(vec![e], vec![]).unwrap();
    assert_eq!(r.validate_against_profile(&profile).unwrap_err(), RegistryError::NoConstruct("line"));
}

#[test]
fn two_entries_on_one_construct_need_distinct_profile_sections() {
    let a = cap(1, "line_a");
    let b = Capability { restriction: "a different restriction", ..cap(2, "line_b") };
    let r = Registry::new(vec![a.clone(), b], vec![]).unwrap();
    // The profile has one section for the construct: the two entries are indistinguishable.
    assert_eq!(
        r.validate_against_profile(&[section(4, "`line`")]).unwrap_err(),
        RegistryError::AmbiguousConstruct { construct: "line", first: "line_a", second: "line_b" }
    );
    // The profile distinguishes them with a section each.
    let b2 = Capability { profile_section: 40, ..cap(2, "line_b") };
    let r = Registry::new(vec![a, b2], vec![]).unwrap();
    assert!(r.validate_against_profile(&[section(4, "`line`"), section(40, "`line`")]).is_ok());
}

#[test]
fn component_entries_are_exempt_from_the_construct_rule_but_must_say_so() {
    let r = Registry::new(vec![cap(1, "line"), component(2, "layer")], vec![]).unwrap();
    assert!(r.validate_against_profile(&[section(4, "`line`")]).is_ok());
    let mut bad = component(2, "layer");
    bad.iso_construct = "line";
    let r = Registry::new(vec![cap(1, "line"), bad], vec![]).unwrap();
    assert_eq!(
        r.validate_against_profile(&[section(4, "`line`")]).unwrap_err(),
        RegistryError::ComponentNamesConstruct("layer")
    );
}

// ---------------------------------------------------------------- class rules

#[test]
fn a_component_can_never_be_registered_as_required() {
    let mut c = component(1, "layer");
    c.requirement = Requirement::Required;
    assert_eq!(Registry::new(vec![c], vec![]).unwrap_err(), RegistryError::ComponentMustBeOptional("layer"));
}

#[test]
fn every_v1_geometry_entry_is_required_and_has_a_construct() {
    for c in Registry::v1().entries().iter().filter(|c| c.class == Class::Geometry) {
        assert_eq!(c.requirement, Requirement::Required, "{}", c.name);
        assert!(!c.iso_construct.is_empty(), "{}", c.name);
        assert!(c.profile_section >= 1, "{}", c.name);
        assert_eq!(c.min_reader, Version::new(1, 0, 0), "{}", c.name);
    }
}

// ---------------------------------------------------------------- negotiation

fn reader(version: Version, implemented: &[CapabilityId]) -> Reader {
    Reader { version, implemented: implemented.to_vec() }
}

fn declared(id: CapabilityId, requirement: Requirement) -> Declared {
    Declared { id, requirement, version: Version::new(1, 0, 0) }
}

#[test]
fn a_reader_missing_a_required_capability_refuses_the_file() {
    let r = Registry::v1();
    let file = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![
            declared(ids::LINE, Requirement::Required),
            declared(ids::CIRCLE, Requirement::Required),
        ],
    };
    let only_lines = reader(Version::new(1, 0, 0), &[ids::LINE]);
    let err = r.negotiate(&only_lines, &file).unwrap_err();
    assert_eq!(err, NegotiationError::MissingRequired { id: ids::CIRCLE, name: Some("circle") });
    assert!(err.to_string().contains("circle"), "{err}");
    // and with the capability it is accepted
    let both = reader(Version::new(1, 0, 0), &[ids::LINE, ids::CIRCLE]);
    assert!(r.negotiate(&both, &file).is_ok());
}

#[test]
fn an_unknown_required_capability_refuses_and_an_unknown_optional_one_is_skipped() {
    let r = Registry::v1();
    let alien = CapabilityId::new(60_000);
    let rdr = reader(Version::new(1, 0, 0), &[ids::LINE]);
    let required = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![declared(alien, Requirement::Required)],
    };
    assert_eq!(
        r.negotiate(&rdr, &required).unwrap_err(),
        NegotiationError::MissingRequired { id: alien, name: None }
    );
    let optional = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![declared(alien, Requirement::Optional)],
    };
    assert_eq!(r.negotiate(&rdr, &optional).unwrap().skipped, vec![alien]);
}

#[test]
fn a_file_cannot_weaken_a_required_capability_by_declaring_it_optional() {
    let r = Registry::v1();
    let file = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![declared(ids::ELLIPSE, Requirement::Optional)],
    };
    let rdr = reader(Version::new(1, 0, 0), &[ids::LINE]);
    assert!(matches!(r.negotiate(&rdr, &file), Err(NegotiationError::MissingRequired { .. })));
}

#[test]
fn a_minimum_reader_version_above_the_readers_is_refused_naming_both() {
    let r = Registry::v1();
    let file = Declaration { min_reader: Version::new(2, 1, 0), capabilities: vec![] };
    let rdr = reader(Version::new(1, 4, 2), &[]);
    let err = r.negotiate(&rdr, &file).unwrap_err();
    assert_eq!(
        err,
        NegotiationError::ReaderTooOld {
            file_requires: Version::new(2, 1, 0),
            reader: Version::new(1, 4, 2)
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("2.1.0") && msg.contains("1.4.2"), "{msg}");
    // equal is fine
    let file = Declaration { min_reader: Version::new(1, 4, 2), capabilities: vec![] };
    assert!(r.negotiate(&rdr, &file).is_ok());
}

#[test]
fn the_version_check_comes_first_and_nothing_is_rendered_partially() {
    // A file that fails both checks reports the reader version, and a refusal carries no partial result.
    let r = Registry::v1();
    let file = Declaration {
        min_reader: Version::new(9, 0, 0),
        capabilities: vec![declared(ids::CIRCLE, Requirement::Required)],
    };
    let rdr = reader(Version::new(1, 0, 0), &[]);
    assert!(matches!(r.negotiate(&rdr, &file), Err(NegotiationError::ReaderTooOld { .. })));
}

#[test]
fn a_file_may_not_declare_a_component_as_required() {
    let r = Registry::new(vec![cap(1, "line"), component(2, "layer")], vec![]).unwrap();
    let file = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![Declared {
            id: CapabilityId::new(2),
            requirement: Requirement::Required,
            version: Version::new(1, 0, 0),
        }],
    };
    let rdr = reader(Version::new(1, 0, 0), &[CapabilityId::new(1)]);
    assert_eq!(
        r.negotiate(&rdr, &file).unwrap_err(),
        NegotiationError::ComponentDeclaredRequired(CapabilityId::new(2))
    );
}

#[test]
fn versions_order_and_print_as_semver() {
    assert!(Version::new(1, 2, 3) < Version::new(1, 10, 0));
    assert!(Version::new(2, 0, 0) > Version::new(1, 99, 99));
    assert_eq!(Version::new(1, 2, 3).to_string(), "1.2.3");
}

// ---------------------------------------------------------------- the generated document

#[test]
fn the_markdown_is_deterministic_and_lists_every_entry_and_retired_id() {
    let retired = vec![Retired { id: CapabilityId::new(2), name: "old_thing", reason: "superseded by 3" }];
    let r = Registry::new(vec![cap(1, "a"), cap(3, "c")], retired).unwrap();
    let md = render_markdown(&r);
    assert_eq!(md, render_markdown(&r));
    for needle in ["| 1 | `a` |", "| 3 | `c` |", "old_thing", "superseded by 3", "generated from"] {
        assert!(md.contains(needle), "{needle:?} missing from\n{md}");
    }
    assert!(md.ends_with('\n') && !md.ends_with("\n\n"));
}

#[test]
fn input_order_does_not_matter_entries_come_out_sorted_and_lookup_works() {
    let retired = vec![Retired { id: CapabilityId::new(2), name: "old", reason: "r" }];
    let r = Registry::new(vec![cap(4, "d"), cap(3, "c"), cap(1, "a")], retired).unwrap();
    let ids: Vec<u32> = r.entries().iter().map(|c| c.id.get()).collect();
    assert_eq!(ids, vec![1, 3, 4]);
    for id in [1, 3, 4] {
        assert_eq!(r.get(CapabilityId::new(id)).map(|c| c.id.get()), Some(id));
    }
    assert!(r.get(CapabilityId::new(2)).is_none());
    assert!(r.get(CapabilityId::new(5)).is_none());
}
