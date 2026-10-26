//! A structure-aware mutator: it changes a field of the container (a header or manifest byte, a directory
//! entry, a capability declaration, a block's bytes) and seals the result again, so the mutant still passes
//! the checksum and the executions go to the parsers behind it.

use crate::layout::Layout;
use crate::reseal;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Values that sit on the edge of a field: zero, one, a sign bit, the largest, and a length.
const EDGES: [u64; 8] = [0, 1, 0x7F, 0x80, 0xFFFF, 0x7FFF_FFFF, 0xFFFF_FFFF, u64::MAX];

fn put(data: &mut [u8], at: usize, rng: &mut Rng) {
    let width = [1usize, 2, 4, 8][rng.below(4)].min(data.len().saturating_sub(at));
    match rng.below(3) {
        0 => {
            let edge = EDGES[rng.below(EDGES.len())].to_le_bytes();
            data[at..at + width].copy_from_slice(&edge[..width]);
        }
        1 => data[at] ^= 1 << rng.below(8),
        _ => data[at] = data[at].wrapping_add(1 + rng.below(255) as u8),
    }
}

/// Mutates `data` in place, growing it to at most `max_len`, from `seed`. An input that does not lay out as a
/// container gets a byte-level mutation instead. The result differs from the input.
pub fn mutate(data: &mut Vec<u8>, max_len: usize, seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..4 {
        rng.next();
    }
    let before = data.clone();
    if data.is_empty() {
        data.push(rng.next() as u8);
        return;
    }
    let Some(layout) = Layout::of(data) else {
        let at = rng.below(data.len());
        put(data, at, &mut rng);
        if rng.below(4) == 0 && data.len() < max_len {
            data.insert(at, rng.next() as u8);
        }
        if *data == before {
            data[at] ^= 0x40;
        }
        return;
    };
    // the regions a mutation can land in: the header, the manifest, the directory, and every block
    let (m0, ml) = layout.manifest_range();
    let (d0, dl) = layout.directory;
    let mut regions: Vec<(usize, usize)> = vec![(0, layout.header.header_len as usize), (m0, ml), (d0, dl)];
    for (i, _) in layout.entries.iter().enumerate() {
        if let Some(b) = layout.stored(data, i) {
            let at = b.as_ptr() as usize - data.as_ptr() as usize;
            if !b.is_empty() {
                regions.push((at, b.len()));
            }
        }
    }
    let n = 1 + rng.below(3);
    for _ in 0..n {
        let (start, len) = regions[rng.below(regions.len())];
        if len == 0 || start >= data.len() {
            continue;
        }
        let at = start + rng.below(len.min(data.len() - start));
        put(data, at, &mut rng);
    }
    if rng.below(16) == 0 && data.len() > 8 {
        // a cut or a tail: the layout no longer holds, and the reader has to say so
        if rng.below(2) == 0 {
            data.truncate(data.len() - 1 - rng.below(8));
        } else if data.len() < max_len {
            data.push(0);
        }
    }
    if *data == before {
        let at = rng.below(data.len());
        data[at] ^= 0x01;
    }
    if let Some(sealed) = reseal(data) {
        *data = sealed;
    }
    if *data == before {
        // a reseal can undo the one change; flip a byte that no checksum covers instead
        let at = rng.below(data.len());
        data[at] ^= 0x01;
    }
    data.truncate(max_len);
}

/// Changes one byte of the manifest and seals the file again; the seeds of the writer's output use it.
pub fn mutate_manifest(data: &mut Vec<u8>, seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
    rng.next();
    if let Some(layout) = Layout::of(data) {
        let (m0, ml) = layout.manifest_range();
        let at = m0 + rng.below(ml);
        put(data, at, &mut rng);
        if let Some(sealed) = reseal(data) {
            *data = sealed;
        }
    }
}
