//! Filters round-trip, codecs refuse a bomb and a wrong length, and the checksums match their
//! published test vectors, so a throughput number is for the real function.

use compress_checksum_spike::*;

fn sample(n: usize) -> Vec<u8> {
    // Smooth doubles, like coordinate columns.
    (0..n / 8)
        .flat_map(|i| (1000.0 + i as f64 * 0.001).to_le_bytes())
        .chain((0..n % 8).map(|b| b as u8))
        .collect()
}

#[test]
fn shuffle_is_its_own_inverse_for_every_length_class() {
    for n in [0usize, 1, 7, 8, 9, 63, 64, 65, 1000, 4097] {
        let d = sample(n);
        assert_eq!(unshuffle8(&shuffle8(&d)), d, "length {n}");
        assert_eq!(shuffle8(&d).len(), d.len());
    }
}

#[test]
fn shuffle_groups_byte_planes() {
    let d: Vec<u8> = (0..16).collect();
    assert_eq!(shuffle8(&d), vec![0, 8, 1, 9, 2, 10, 3, 11, 4, 12, 5, 13, 6, 14, 7, 15]);
}

fn codecs() -> Vec<Codec> {
    #[allow(unused_mut)]
    let mut v = vec![Codec::Deflate1, Codec::Deflate6, Codec::Lz4];
    #[cfg(not(target_arch = "wasm32"))]
    v.extend([Codec::Zstd1, Codec::Zstd3]);
    v
}

#[test]
fn every_codec_round_trips() {
    let d = sample(40_000);
    for c in codecs() {
        assert_eq!(c.decompress(&c.compress(&d), d.len()).as_deref(), Some(&d[..]), "{}", c.name());
    }
}

#[test]
fn a_wrong_expected_length_is_refused() {
    let d = sample(10_000);
    for c in codecs() {
        let z = c.compress(&d);
        assert!(c.decompress(&z, d.len() - 1).is_none(), "{} accepted a short length", c.name());
        assert!(c.decompress(&z, d.len() + 1).is_none(), "{} accepted a long length", c.name());
    }
}

#[test]
fn a_decompression_bomb_is_not_expanded() {
    // 64 MB of zeros compress by well over 100 to 1; claiming a 1 KB block must fail without
    // producing 64 MB.
    let bomb = Codec::Deflate1.compress(&vec![0u8; 64 << 20]);
    assert!(bomb.len() * 100 < (64 << 20), "expected a ratio above 100, got {} bytes", bomb.len());
    assert!(Codec::Deflate1.decompress(&bomb, 1024).is_none());
    let bomb = Codec::Lz4.compress(&vec![0u8; 64 << 20]);
    assert!(bomb.len() * 100 < (64 << 20));
    assert!(Codec::Lz4.decompress(&bomb, 1024).is_none());
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    let junk: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    for c in codecs() {
        let _ = c.decompress(&junk, 4096);
    }
}

#[test]
fn checksums_match_published_vectors() {
    assert_eq!(checksum("crc32 (crc32fast)", b"123456789"), 0xCBF43926);
    assert_eq!(checksum("crc32c", b"123456789"), 0xE3069283);
    assert_eq!(checksum("xxh3-64", b""), 0x2D06800538D394C2);
    assert_eq!(checksum("fnv-1a 64", b""), 0xcbf29ce484222325);
    assert_eq!(checksum("fnv-1a 64", b"a"), 0xaf63dc4c8601ec8c);
    // SHA-256("abc") = ba7816bf 8f01cfea ...; the first eight bytes, read little-endian.
    assert_eq!(
        checksum("sha-256 (first 8 bytes)", b"abc"),
        u64::from_le_bytes([0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea])
    );
}

#[test]
fn a_single_flipped_bit_changes_every_checksum() {
    let d = sample(33_000);
    let mut e = d.clone();
    e[12_345] ^= 0x10;
    for k in CHECKSUMS {
        assert_ne!(checksum(k, &d), checksum(k, &e), "{k}");
    }
}
