//! `expected.math.json`: the mathematics a fixture was authored from, before any DWG existed.

use crate::error::Result;
use crate::scalar::Scalar;
use pvec_capabilities::EvidenceClass;

/// A parsed JSON tree in which every `{decimal, f64_hex}` object is already a [`Scalar`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// A lossless real.
    Scalar(Scalar),
    /// An exact integer (a degree, a multiplicity).
    Int(i64),
    /// A flag.
    Bool(bool),
    /// Text.
    Str(String),
    /// A list.
    List(Vec<Node>),
    /// A map, in file order.
    Map(Vec<(String, Node)>),
}

impl Node {
    /// A member of a map.
    pub fn get(&self, _key: &str) -> Option<&Node> {
        todo!("Node::get")
    }
    /// The scalar, if this is one.
    pub fn as_scalar(&self) -> Option<&Scalar> {
        todo!("Node::as_scalar")
    }
    /// The elements, if this is a list.
    pub fn as_list(&self) -> Option<&[Node]> {
        todo!("Node::as_list")
    }
    /// Every scalar below this node, with a path such as `entities[0].geometry.center[1]`.
    pub fn scalars<'a>(&'a self, _path: &str, _out: &mut Vec<(String, &'a Scalar)>) {
        todo!("Node::scalars")
    }
}

/// The curve classes of EPIC O's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityClass {
    /// `line`.
    Line,
    /// `polyline`.
    Polyline,
    /// `circle`.
    Circle,
    /// `circular_arc`.
    CircularArc,
    /// `ellipse`.
    Ellipse,
    /// `elliptical_arc`.
    EllipticalArc,
    /// `bezier`.
    Bezier,
    /// `bspline`.
    Bspline,
    /// `nurbs`, a rational B-spline.
    Nurbs,
}

/// One intended entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    /// The entity id inside the record.
    pub id: String,
    /// Its class.
    pub class: EntityClass,
    /// The class's geometry fields.
    pub geometry: Node,
    /// The whole entity, for the fields this loader does not interpret (transform, world, closure).
    pub node: Node,
}

/// The drawing's units and the exact factor to metres.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Units {
    /// The source unit name.
    pub name: String,
    /// The factor to the canonical unit.
    pub scale_to_canonical: Scalar,
}

/// One fixture's intended mathematics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedMath {
    /// The fixture id the record is for.
    pub fixture_id: String,
    /// The evidence class the record declares.
    pub evidence_class: EvidenceClass,
    /// Units.
    pub units: Units,
    /// The entities, in file order.
    pub entities: Vec<Entity>,
    /// The whole document.
    pub node: Node,
}

impl ExpectedMath {
    /// Parses and validates one record. Every scalar is checked on the way in.
    pub fn parse(_bytes: &[u8]) -> Result<ExpectedMath> {
        todo!("ExpectedMath::parse")
    }
    /// Every scalar in the record, with its path.
    pub fn scalars(&self) -> Vec<(String, &Scalar)> {
        todo!("ExpectedMath::scalars")
    }
}
