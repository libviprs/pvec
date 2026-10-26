#![no_main]

use libfuzzer_sys::{fuzz_mutator, fuzz_target};

fuzz_target!(|data: &[u8]| {
    pvec_fuzz::fuzz_header(data);
});

// Structure-aware: change a field of the container and seal it again, so the input still passes the checksum.
fuzz_mutator!(|data: &mut [u8], size: usize, max_size: usize, seed: u32| {
    let mut bytes = data[..size].to_vec();
    pvec_fuzz::mutate::mutate(&mut bytes, max_size, u64::from(seed));
    let n = bytes.len().min(max_size);
    data[..n].copy_from_slice(&bytes[..n]);
    n
});
