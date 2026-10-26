//! A structure-aware mutator: it changes a field of the container (a manifest field, a directory entry, a
//! capability declaration, a block's bytes) and seals the result again, so the mutant still passes the
//! checksum and the executions go to the parsers behind it.

/// Mutates `data` in place, growing it to at most `max_len`, from `seed`. An input that does not lay out as a
/// container gets a byte-level mutation instead.
pub fn mutate(_data: &mut Vec<u8>, _max_len: usize, _seed: u64) {
    todo!("P7.8")
}
