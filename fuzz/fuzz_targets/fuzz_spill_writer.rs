#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    pvec_fuzz::fuzz_spill_writer(data);
});
