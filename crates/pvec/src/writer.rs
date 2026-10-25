use crate::components::Stream;
use crate::{Bounds2, Curve, CurveRef, EntityId, Manifest, PvecError};
use pvec_container::Bounds;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriteError};

/// Writes a PVec file from semantic entities.
///
/// The caller supplies curves and ids. It never names a cell or a block: the encoding is the
/// writer's business.
#[derive(Debug)]
pub struct PvecWriter {
    inner: ContainerWriter,
}

fn refused(e: WriteError) -> PvecError {
    PvecError::Write { rule: e.rule().to_string() }
}

impl PvecWriter {
    /// Starts a file with this manifest.
    pub fn new(manifest: Manifest) -> Result<PvecWriter, PvecError> {
        Ok(PvecWriter { inner: ContainerWriter::new(manifest.config()?) })
    }

    /// Adds an entity. Its bounds are derived from the curve. A composite goes through
    /// [`PvecWriter::push_composite`], which needs its segments by reference.
    pub fn push(&mut self, id: EntityId, curve: &Curve) -> Result<(), PvecError> {
        self.inner.push_curve(id, 0, 0, curve).map_err(refused)
    }

    /// Adds a composite whose segments are other entities already pushed, with the bounds of the whole.
    pub fn push_composite(
        &mut self,
        id: EntityId,
        segments: &[CurveRef],
        bounds: Bounds2,
        closed: bool,
    ) -> Result<(), PvecError> {
        let meta = EntityMeta { id, style: 0, cell: 0, bounds: Bounds { min: bounds.min, max: bounds.max } };
        let ids: Vec<EntityId> = segments.iter().map(|s| s.entity).collect();
        self.inner.push_composite(meta, &ids, None, closed).map_err(refused)
    }

    /// Attaches a component stream, which refers to entities by id.
    pub fn components(&mut self, stream: Stream) -> Result<(), PvecError> {
        self.inner.push_stream(stream).map_err(refused)
    }

    /// Seals the file and returns its bytes.
    pub fn finish(self) -> Result<Vec<u8>, PvecError> {
        self.inner.finish().map_err(refused)
    }
}
