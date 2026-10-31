#![no_main]

use libfuzzer_sys::{fuzz_mutator, fuzz_target};

fuzz_target!(|data: &[u8]| {
    pvec_fuzz::fuzz_raster_index(data);
});

// Structure-aware: change a field of the index and keep its length, so the counts in the header still add up
// and the input gets past the length check.
fuzz_mutator!(|data: &mut [u8], size: usize, max_size: usize, seed: u32| {
    let _ = max_size;
    pvec_fuzz::mutate::mutate_index(&mut data[..size], u64::from(seed));
    size
});
