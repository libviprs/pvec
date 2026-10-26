use crate::{Curve, EntityId, Manifest, PvecError};
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;

/// Reads a PVec file held in memory.
pub struct PvecReader {
    inner: ContainerReader<MemorySource<Vec<u8>>>,
    manifest: Manifest,
}

impl PvecReader {
    /// Opens a file, checking the header, manifest and directory.
    pub fn open(bytes: Vec<u8>) -> Result<PvecReader, PvecError> {
        let inner = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default())
            .map_err(|e| PvecError::Read { rule: e.rule() })?;
        let manifest = Manifest::from_container(inner.manifest());
        Ok(PvecReader { inner, manifest })
    }

    /// What the file says about itself.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Reads and checks every block and returns the number of entities in the file.
    pub fn entity_count(&self) -> Result<u64, PvecError> {
        let v = self.inner.validate().map_err(|e| PvecError::Read { rule: e.rule() })?;
        Ok(v.entities)
    }

    /// Every entity of the file as a curve, in ascending entity id order.
    pub fn entities(&self) -> Result<Vec<(EntityId, Curve)>, PvecError> {
        Err(PvecError::Read { rule: "not implemented".to_string() })
    }
}
