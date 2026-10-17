//! Every candidate must reproduce the fixtures exactly, agree on the touch sum, and survive hostile
//! bytes without panicking.

use layouts_spike::common::*;
use layouts_spike::fbs_obj::FbObj;
use layouts_spike::fbs_soa::{FbSoa, FbSoaRaw};
use layouts_spike::packed::Packed;
use layouts_spike::rk::Rkyv;
use pvec_fixtures::{Drawing, generate};

fn reference_touch(d: &Drawing) -> u64 {
    d.entities.iter().fold(0u64, |a, e| a.wrapping_add(touch_entity(e)))
}

fn check<C: Codec>(d: &Drawing) {
    let bl = blocks(d);
    let file = Aligned::from_bytes(&build_file::<C>(&bl));
    let bytes = file.as_bytes();
    // Exact reproduction: same entities, bit for bit (PartialEq on f64 is exact for these values),
    // and the published content checksum.
    let back = decode_file::<C>(bytes).unwrap_or_else(|e| panic!("{} decode: {e}", C::NAME));
    assert_eq!(back, d.entities, "{} did not reproduce {}", C::NAME, d.name);
    let rebuilt = Drawing { name: d.name, extent: d.extent, entities: back };
    assert_eq!(rebuilt.checksum(), d.checksum(), "{} checksum on {}", C::NAME, d.name);
    // The touch workload agrees with the reference computed from the source data.
    let (sum, n) = touch_file::<C>(bytes).unwrap_or_else(|e| panic!("{} touch: {e}", C::NAME));
    assert_eq!(n as usize, d.entities.len());
    assert_eq!(sum, reference_touch(d), "{} touch sum on {}", C::NAME, d.name);
}

fn all(d: &Drawing) {
    check::<Packed>(d);
    check::<FbSoa>(d);
    check::<FbSoaRaw>(d);
    check::<Rkyv>(d);
    check::<FbObj>(d);
}

#[test]
fn small_synthetic_roundtrips_in_every_layout() {
    all(&generate::small_synthetic());
}

#[test]
fn spline_heavy_roundtrips_in_every_layout() {
    all(&generate::spline_heavy());
}

#[test]
fn straddler_roundtrips_in_every_layout() {
    all(&generate::straddler());
}

fn lcg(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 33
}

/// Flips bytes and truncates; a decoder may accept or reject, but must never panic or run away.
fn hostile<C: Codec>() {
    let d = generate::small_synthetic();
    let bl = blocks(&d);
    let first = C::encode_block(&bl[0]);
    let mut seed = 7u64;
    for round in 0..400 {
        let mut b = first.clone();
        match round % 4 {
            0 => b.truncate((lcg(&mut seed) as usize) % b.len()),
            1 => {
                for _ in 0..4 {
                    let i = (lcg(&mut seed) as usize) % b.len();
                    b[i] ^= 1 << (lcg(&mut seed) % 8);
                }
            }
            2 => {
                // Overwrite a 4 byte word with a huge count or offset.
                let i = (lcg(&mut seed) as usize) % b.len().saturating_sub(4).max(1);
                b[i..i + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            }
            _ => b = (0..b.len()).map(|_| lcg(&mut seed) as u8).collect(),
        }
        let a = Aligned::from_bytes(&b);
        let _ = C::touch(a.as_bytes());
        let _ = C::decode(a.as_bytes());
    }
}

#[test]
fn hostile_bytes_never_panic_candidate_b() {
    hostile::<Packed>();
}
#[test]
fn hostile_bytes_never_panic_candidate_a() {
    hostile::<FbSoa>();
    hostile::<FbSoaRaw>();
}
#[test]
fn hostile_bytes_never_panic_candidate_c() {
    hostile::<Rkyv>();
}
#[test]
fn hostile_bytes_never_panic_baseline() {
    hostile::<FbObj>();
}

#[test]
fn viewport_ranges_cover_exactly_the_hit_blocks() {
    let d = generate::small_synthetic();
    let bl = blocks(&d);
    let file = Aligned::from_bytes(&build_file::<Packed>(&bl));
    let dir = parse_dir(file.as_bytes()).unwrap();
    let all_rect = d.viewport(&pvec_fixtures::viewport_script()[0]);
    let hit = blocks_hit(&dir, &all_rect);
    assert_eq!(hit.len(), dir.len(), "the whole-drawing viewport hits every block");
    let r = ranges(&dir, &hit);
    assert_eq!(r.len(), 1, "adjacent blocks merge into one range");
    // An entity that is inside a viewport must be in a block the viewport hits.
    for step in pvec_fixtures::viewport_script() {
        let rect = d.viewport(&step);
        let hit = blocks_hit(&dir, &rect);
        let decoded: Vec<_> = hit
            .iter()
            .flat_map(|&i| Packed::decode(block_bytes(file.as_bytes(), &dir[i])).unwrap())
            .collect();
        let want = d.entities.iter().filter(|e| pvec_fixtures::overlaps(&e.geom.bbox(), &rect)).count();
        let got = decoded.iter().filter(|e| pvec_fixtures::overlaps(&e.geom.bbox(), &rect)).count();
        assert_eq!(got, want, "step {}", step.name);
    }
}

#[test]
fn padding_per_spline_curve_is_the_documented_six_bytes() {
    // 16 byte record header minus 2 bytes of degree and rational flag, 4 of each count (x2 = 8)
    // leaves 6 bytes that carry nothing.
    assert_eq!(layouts_spike::packed::spline_padding_bytes_per_curve(), 6);
    assert_eq!(layouts_spike::packed::SPLINE_HEADER - 2 - 8, 6);
}
