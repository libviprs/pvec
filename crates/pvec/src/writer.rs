use crate::components::Stream;
use crate::{Bounds2, Curve, CurveRef, EntityId, Manifest, PvecError};
use pvec_container::Bounds;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriteError};
#[cfg(feature = "unstable-stream")]
use pvec_container::writer::WriterConfig;

/// Writes a PVec file from semantic entities.
///
/// The caller supplies curves and ids. It never names a cell or a block: the encoding is the
/// writer's business.
#[derive(Debug)]
pub struct PvecWriter {
    pub(crate) inner: ContainerWriter,
    /// What `inner` was made with, for a writer that is switched to keep its pushes in files.
    #[cfg(feature = "unstable-stream")]
    pub(crate) config: WriterConfig,
    /// Whether anything has been pushed, which is when it is too late to switch to files.
    #[cfg(feature = "unstable-stream")]
    pub(crate) touched: bool,
    /// Set by `PvecWriter::new_indexed`: what `finish` needs to lay the file out again with a spatial directory.
    #[cfg(feature = "unstable-stream")]
    pub(crate) index: Option<crate::unstable::Indexing>,
    /// Set by `PvecWriter::with_spill`: the pushes are in files, and `finish_to` builds from them.
    #[cfg(feature = "unstable-stream")]
    pub(crate) spill: Option<Box<crate::unstable::Spill>>,
}

pub(crate) fn refused(e: WriteError) -> PvecError {
    PvecError::Write { rule: e.rule().to_string() }
}

impl PvecWriter {
    /// Starts a file with this manifest.
    pub fn new(manifest: Manifest) -> Result<PvecWriter, PvecError> {
        let config = manifest.config()?;
        #[cfg(not(feature = "unstable-stream"))]
        let inner = ContainerWriter::new(config);
        #[cfg(feature = "unstable-stream")]
        let inner = ContainerWriter::new(config.clone());
        Ok(PvecWriter {
            inner,
            #[cfg(feature = "unstable-stream")]
            config,
            #[cfg(feature = "unstable-stream")]
            touched: false,
            #[cfg(feature = "unstable-stream")]
            index: None,
            #[cfg(feature = "unstable-stream")]
            spill: None,
        })
    }

    /// Adds an entity. Its bounds are derived from the curve. A composite goes through
    /// [`PvecWriter::push_composite`], which needs its segments by reference.
    pub fn push(&mut self, id: EntityId, curve: &Curve) -> Result<(), PvecError> {
        #[cfg(feature = "unstable-stream")]
        {
            self.touched = true;
            if let Some(spill) = &mut self.spill {
                return spill.push(id, curve);
            }
        }
        self.inner.push_curve(id, 0, 0, curve).map_err(refused)?;
        #[cfg(feature = "unstable-stream")]
        if let Some(index) = &mut self.index {
            index.log.push(crate::unstable::Op::Curve(id, curve.clone()));
        }
        Ok(())
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
        #[cfg(feature = "unstable-stream")]
        {
            self.touched = true;
            if let Some(spill) = &mut self.spill {
                return spill.push_composite(meta, &ids, closed);
            }
        }
        self.inner.push_composite(meta, &ids, None, closed).map_err(refused)?;
        #[cfg(feature = "unstable-stream")]
        if let Some(index) = &mut self.index {
            index.log.push(crate::unstable::Op::Composite { id, segments: ids, bounds, closed });
        }
        Ok(())
    }

    /// Attaches a component stream, which refers to entities by id.
    pub fn components(&mut self, stream: Stream) -> Result<(), PvecError> {
        #[cfg(feature = "unstable-stream")]
        {
            self.touched = true;
            if let Some(spill) = &mut self.spill {
                return spill.push_stream(&stream);
            }
        }
        #[cfg(feature = "unstable-stream")]
        if let Some(index) = &mut self.index {
            index.log.push(crate::unstable::Op::Components(stream.clone()));
        }
        self.inner.push_stream(stream).map_err(refused)
    }

    /// Seals the file and returns its bytes.
    pub fn finish(self) -> Result<Vec<u8>, PvecError> {
        #[cfg(feature = "unstable-stream")]
        if self.spill.is_some() {
            let mut out = Vec::new();
            self.finish_to(&mut out)?;
            return Ok(out);
        }
        #[cfg(feature = "unstable-stream")]
        if let Some(index) = self.index {
            return index.lay_out(self.inner);
        }
        self.inner.finish().map_err(refused)
    }
}
