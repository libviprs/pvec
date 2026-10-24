//! The checks of the reader, sans IO (issue P4.5). STUB for the red commit: nothing is implemented.
#![allow(dead_code)]

use crate::blocks::advanced::AdvancedView;
use crate::directory::BlockEntry;
use crate::limits::{Accountant, Limits, Reservation};
use crate::reader::{BlockId, DecodedBlock, RangeRequest, ReadError, Validated};
use crate::{Header, Manifest};
use std::sync::Arc;

fn unimplemented_read<T>() -> Result<T, ReadError> {
    Err(ReadError::Layout { rule: "NOT-IMPLEMENTED", detail: "the reader is not written yet".into() })
}

pub(crate) struct Core {
    pub header: Header,
    pub manifest: Manifest,
    pub entries: Vec<BlockEntry>,
}

impl Core {
    pub fn entry(&self, _id: BlockId) -> Result<&BlockEntry, ReadError> {
        unimplemented_read()
    }
    pub fn range_of(&self, _id: BlockId) -> Result<(u64, u32), ReadError> {
        unimplemented_read()
    }
    pub fn slice_of<'a>(&self, _id: BlockId, _at: u64, _bytes: &'a [u8]) -> Result<&'a [u8], ReadError> {
        unimplemented_read()
    }
    pub fn plan(&self, _ids: &[BlockId], _gap: u32, _max: u32) -> Result<Vec<RangeRequest>, ReadError> {
        unimplemented_read()
    }
    pub fn decode(&self, _id: BlockId, _stored: &[u8], _r: Reservation) -> Result<DecodedBlock, ReadError> {
        unimplemented_read()
    }
    pub fn advanced_ids(&self) -> Result<Option<Vec<BlockId>>, ReadError> {
        unimplemented_read()
    }
}

pub(crate) enum Need {
    Gap { offset: u64, len: u32 },
    Block(BlockId),
}

pub(crate) struct Opening;

impl Opening {
    pub fn new(_artifact_len: u64, _limits: &Limits) -> Result<Opening, ReadError> {
        unimplemented_read()
    }
    pub fn request(&self) -> Option<(u64, u32)> {
        None
    }
    pub fn feed(&mut self, _bytes: &[u8]) -> Result<(), ReadError> {
        unimplemented_read()
    }
    pub fn finish(self) -> Result<Core, ReadError> {
        unimplemented_read()
    }
}

pub(crate) struct Validation<'a> {
    _core: &'a Core,
}

impl<'a> Validation<'a> {
    pub fn new(_core: &'a Core, _limits: &Limits, _acct: &Accountant) -> Result<Validation<'a>, ReadError> {
        unimplemented_read()
    }
    pub fn next_need(&mut self) -> Option<Need> {
        None
    }
    pub fn feed_gap(&mut self, _offset: u64, _bytes: &[u8]) -> Result<(), ReadError> {
        unimplemented_read()
    }
    pub fn feed_block(&mut self, _block: Arc<DecodedBlock>) -> Result<(), ReadError> {
        unimplemented_read()
    }
    pub fn finish(self) -> Result<Validated, ReadError> {
        unimplemented_read()
    }
}

pub(crate) fn check_nesting(_view: &AdvancedView<'_>, _max_depth: usize) -> Result<(), ReadError> {
    unimplemented_read()
}

pub(crate) const READER_RULES: &[(&str, &str)] = &[];

pub(crate) fn render_reader_doc() -> String {
    String::new()
}
