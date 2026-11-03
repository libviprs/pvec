//! Issue 261: NP-06 on a line segment decides with `detmath::hypot`, so the accept or reject of an
//! entity does not depend on the C library of the machine that wrote the file.

use pvec_container::blocks::hot::{HotBlockBuilder, HotError, HotKind};
use pvec_entity::EntityId;

#[test]
fn a_segment_exactly_the_floor_long_is_refused_on_every_machine() {
    // detmath::hypot of this vector is the NP-06 floor itself (refused); x86_64 glibc's hypot says one
    // ulp more, which used to accept it there and refuse it elsewhere.
    let d = [f64::from_bits(0x3d65_ac3a_dd10_d876), f64::from_bits(0x3d6b_b78b_f4a3_9cf4)];
    let mut b = HotBlockBuilder::new(HotKind::Line);
    let e = b.push_line(EntityId::new(1).unwrap(), 0, [0.0, 0.0], d).unwrap_err();
    assert!(matches!(e, HotError::DegenerateLength { .. }), "{e:?}");
}
