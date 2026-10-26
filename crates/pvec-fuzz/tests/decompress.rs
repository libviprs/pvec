//! Issue P7.8: no corpus entry can exceed the configured expansion ratio, and a bomb is a clean refusal.

use pvec_container::Limits;
use pvec_container::compress::{Codec, CodecPolicy};
use pvec_fuzz::{Boundary, decompress_boundary, fuzz_decompress_boundary};

fn framed(codec: u8, declared: u32, stored: &[u8]) -> Vec<u8> {
    let mut v = vec![codec];
    v.extend_from_slice(&declared.to_le_bytes());
    v.extend_from_slice(stored);
    v
}

#[test]
fn a_bomb_is_a_refusal_for_every_codec_and_never_a_decode() {
    for codec in [Codec::None, Codec::Lz4, Codec::Deflate] {
        let bomb = framed(codec.id(), u32::MAX, &[0u8; 24]);
        assert_eq!(decompress_boundary(&bomb), Boundary::Refused("LIMIT-01".into()), "{codec:?}");
        assert_eq!(fuzz_decompress_boundary(&bomb), "LIMIT-01");
    }
}

#[test]
fn a_stream_inside_the_ratio_decodes_to_exactly_the_declared_length() {
    let raw: Vec<u8> = (0..4000u32).map(|i| (i % 17) as u8).collect();
    for (codec, stored) in [
        (Codec::Lz4, pvec_container::compress::compress(Codec::Lz4, &raw)),
        (Codec::Deflate, pvec_container::compress::compress(Codec::Deflate, &raw)),
    ] {
        assert!(raw.len() <= stored.len() * Limits::DEFAULT.max_expansion_ratio as usize);
        assert_eq!(
            decompress_boundary(&framed(codec.id(), raw.len() as u32, &stored)),
            Boundary::Decoded(raw.len())
        );
    }
}

#[test]
fn a_stream_that_expands_beyond_the_ratio_is_refused_even_when_it_is_well_formed() {
    let raw = vec![0u8; 1 << 20];
    let stored = pvec_container::compress::compress(Codec::Deflate, &raw);
    assert!(
        raw.len() > stored.len() * Limits::DEFAULT.max_expansion_ratio as usize,
        "a megabyte of zeros is a bomb"
    );
    assert_eq!(
        decompress_boundary(&framed(Codec::Deflate.id(), raw.len() as u32, &stored)),
        Boundary::Refused("LIMIT-01".into())
    );
}

#[test]
fn input_shorter_than_its_frame_is_not_framed() {
    for n in 0..5 {
        assert_eq!(decompress_boundary(&vec![1; n]), Boundary::NotFramed);
    }
}

#[test]
fn whatever_the_bytes_say_a_decode_never_exceeds_the_ratio_or_the_declared_length() {
    let policy = CodecPolicy::NONE;
    let _ = policy;
    let mut x = 0xD1B5_4A32_D192_ED03u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..4000 {
        let len = (next() % 200) as usize;
        let mut v: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if v.len() >= 5 {
            v[0] %= 4;
            if next() % 3 == 0 {
                let d = (next() % 4096) as u32;
                v[1..5].copy_from_slice(&d.to_le_bytes());
            }
        }
        if let Boundary::Decoded(n) = decompress_boundary(&v) {
            let stored = v.len() - 5;
            assert!(n <= stored * Limits::DEFAULT.max_expansion_ratio as usize, "{n} from {stored}");
            assert_eq!(n as u32, u32::from_le_bytes([v[1], v[2], v[3], v[4]]));
        }
    }
}
