//! Space-filling curve keys.

/// Hilbert curve index of cell `(x, y)` on a `2^order` square.
pub fn hilbert(order: u32, mut x: u32, mut y: u32) -> u64 {
    let n = 1u32 << order;
    let mut d = 0u64;
    let mut s = n >> 1;
    while s > 0 {
        let rx = u32::from(x & s != 0);
        let ry = u32::from(y & s != 0);
        d += u64::from(s) * u64::from(s) * u64::from((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s >>= 1;
    }
    d
}

/// Morton (Z-order) key of `(x, y)`, 16 bits per axis.
pub fn morton(x: u32, y: u32) -> u64 {
    fn spread(v: u32) -> u64 {
        let mut v = u64::from(v & 0xffff);
        v = (v | (v << 8)) & 0x00ff00ff;
        v = (v | (v << 4)) & 0x0f0f0f0f;
        v = (v | (v << 2)) & 0x33333333;
        v = (v | (v << 1)) & 0x55555555;
        v
    }
    spread(x) | (spread(y) << 1)
}
