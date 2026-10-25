use crate::{Bounds2, CapabilityId, PvecError};
use pvec_container::{Bounds, LengthUnit, Metadata};

/// What a file says about itself, in terms a consumer can use.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Metres in one source unit. Must be one of the units the format defines.
    pub metres_per_unit: f64,
    /// Source bounds.
    pub bounds: Bounds2,
    /// The capabilities the file declares.
    pub capabilities: Vec<CapabilityId>,
    /// Producer id.
    pub producer_id: String,
    /// Producer version.
    pub producer_version: String,
}

impl Manifest {
    /// Maps to the container's config (conversion layer between the facade and the encoding).
    pub(crate) fn config(&self) -> Result<pvec_container::writer::WriterConfig, PvecError> {
        let unit = LengthUnit::ALL
            .into_iter()
            .find(|u| u.metres_per_unit().to_bits() == self.metres_per_unit.to_bits())
            .ok_or(PvecError::UnknownUnit { metres_per_unit: self.metres_per_unit })?;
        Ok(pvec_container::writer::WriterConfig {
            unit,
            metadata: Metadata {
                producer_id: self.producer_id.clone(),
                producer_version: self.producer_version.clone(),
                corpus_revision: None,
            },
            bounds: Some(Bounds { min: self.bounds.min, max: self.bounds.max }),
            capabilities: self.capabilities.clone(),
        })
    }

    pub(crate) fn from_container(m: &pvec_container::Manifest) -> Manifest {
        Manifest {
            metres_per_unit: m.unit.metres_per_unit(),
            bounds: Bounds2 { min: m.bounds.min, max: m.bounds.max },
            capabilities: m.capabilities.iter().map(|d| d.id).collect(),
            producer_id: m.metadata.producer_id.clone(),
            producer_version: m.metadata.producer_version.clone(),
        }
    }
}
