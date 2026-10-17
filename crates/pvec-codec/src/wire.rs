//! A bounds-checked little-endian reader. Nothing here indexes by a value from the file: every read
//! goes through `take`, and every count is compared with the bytes that are really left before it
//! can size an allocation.

use crate::error::DecodeError;

pub(crate) type Res<T> = Result<T, DecodeError>;

pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub(crate) fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        if n > self.remaining() {
            return Err(DecodeError::Truncated);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub(crate) fn u16(&mut self) -> Res<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Res<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Skips to the next multiple of 8, which must be zero bytes (the format is canonical).
    pub(crate) fn skip_padding(&mut self) -> Res<()> {
        let pad = (8 - self.pos % 8) % 8;
        if self.take(pad)?.iter().any(|b| *b != 0) {
            return Err(DecodeError::NonZeroPadding);
        }
        Ok(())
    }

    /// `n` little-endian `u32`, then padding. The size is checked against the bytes left first.
    pub(crate) fn u32s(&mut self, n: usize) -> Res<Vec<u32>> {
        let bytes = n.checked_mul(4).ok_or(DecodeError::Truncated)?;
        let raw = self.take(bytes)?;
        let out = raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        self.skip_padding()?;
        Ok(out)
    }

    /// `n` little-endian `f64`. Always a multiple of 8 bytes, so no padding follows.
    pub(crate) fn f64s(&mut self, n: usize) -> Res<Vec<f64>> {
        let bytes = n.checked_mul(8).ok_or(DecodeError::Truncated)?;
        let raw = self.take(bytes)?;
        Ok(raw
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    /// `n` bytes, then padding.
    pub(crate) fn u8s(&mut self, n: usize) -> Res<&'a [u8]> {
        let raw = self.take(n)?;
        self.skip_padding()?;
        Ok(raw)
    }
}

/// An append-only little-endian writer with 8-byte padding.
#[derive(Default)]
pub(crate) struct Writer {
    pub(crate) buf: Vec<u8>,
}

impl Writer {
    pub(crate) fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn pad8(&mut self) {
        while self.buf.len() % 8 != 0 {
            self.buf.push(0);
        }
    }

    pub(crate) fn u32s(&mut self, v: impl IntoIterator<Item = u32>) {
        for x in v {
            self.u32(x);
        }
        self.pad8();
    }

    pub(crate) fn f64s(&mut self, v: impl IntoIterator<Item = f64>) {
        for x in v {
            self.f64(x);
        }
    }

    pub(crate) fn u8s(&mut self, v: impl IntoIterator<Item = u8>) {
        self.buf.extend(v);
        self.pad8();
    }
}
