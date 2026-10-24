//! `expected.math.json`: the mathematics a fixture was authored from, before any DWG existed.

use crate::error::{CorpusError, Result};
use crate::json::{self, Value};
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
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// The scalar, if this is one.
    pub fn as_scalar(&self) -> Option<&Scalar> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }
    /// The elements, if this is a list.
    pub fn as_list(&self) -> Option<&[Node]> {
        match self {
            Node::List(l) => Some(l),
            _ => None,
        }
    }
    /// Every scalar below this node, with a path such as `entities[0].geometry.center[1]`.
    pub fn scalars<'a>(&'a self, path: &str, out: &mut Vec<(String, &'a Scalar)>) {
        match self {
            Node::Scalar(s) => out.push((path.to_string(), s)),
            Node::List(items) => {
                for (i, n) in items.iter().enumerate() {
                    n.scalars(&format!("{path}[{i}]"), out);
                }
            }
            Node::Map(members) => {
                for (k, n) in members {
                    let child = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                    n.scalars(&child, out);
                }
            }
            Node::Int(_) | Node::Bool(_) | Node::Str(_) => {}
        }
    }

    /// Converts a JSON tree. An object with a `decimal` or an `f64_hex` member has to be a whole
    /// scalar; a `null` has no place in a record.
    fn from_json(at: &str, v: &Value) -> Result<Node> {
        Ok(match v {
            Value::Null => {
                return Err(CorpusError::Invalid {
                    what: at.to_string(),
                    why: "null has no place in a record".to_string(),
                });
            }
            Value::Bool(b) => Node::Bool(*b),
            Value::Int(i) => Node::Int(*i),
            Value::Str(s) => Node::Str(s.clone()),
            Value::Arr(items) => {
                let mut out = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    out.push(Node::from_json(&format!("{at}[{i}]"), item)?);
                }
                Node::List(out)
            }
            Value::Obj(members) => {
                if v.get("decimal").is_some() || v.get("f64_hex").is_some() {
                    Node::Scalar(Scalar::from_json(at, v)?)
                } else {
                    let mut out = Vec::with_capacity(members.len());
                    for (k, item) in members {
                        let child = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                        out.push((k.clone(), Node::from_json(&child, item)?));
                    }
                    Node::Map(out)
                }
            }
        })
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

const SCHEMA: &str = "pvec-gold/expected.math/1";

fn is_fixture_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

impl EntityClass {
    fn parse(name: &str) -> Option<EntityClass> {
        Some(match name {
            "line" => EntityClass::Line,
            "polyline" => EntityClass::Polyline,
            "circle" => EntityClass::Circle,
            "circular_arc" => EntityClass::CircularArc,
            "ellipse" => EntityClass::Ellipse,
            "elliptical_arc" => EntityClass::EllipticalArc,
            "bezier" => EntityClass::Bezier,
            "bspline" => EntityClass::Bspline,
            "nurbs" => EntityClass::Nurbs,
            _ => return None,
        })
    }

    /// The geometry fields the record's table gives each class.
    fn required_fields(self) -> &'static [&'static str] {
        match self {
            EntityClass::Line => &["start", "end"],
            EntityClass::Polyline => &["vertices", "bulges"],
            EntityClass::Circle => &["center", "normal", "x_axis", "radius"],
            EntityClass::CircularArc => {
                &["center", "normal", "x_axis", "radius", "start_angle", "sweep_angle"]
            }
            EntityClass::Ellipse => &["center", "normal", "major_axis", "semi_major", "semi_minor"],
            EntityClass::EllipticalArc => {
                &["center", "normal", "major_axis", "semi_major", "semi_minor", "start_angle", "sweep_angle"]
            }
            EntityClass::Bezier => &["degree", "control_points"],
            EntityClass::Bspline => &["degree", "control_points", "knots"],
            EntityClass::Nurbs => &["degree", "control_points", "knots", "weights", "weights_all_unit"],
        }
    }
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
    pub fn parse(bytes: &[u8]) -> Result<ExpectedMath> {
        let root = Node::from_json("", &json::parse(bytes)?)?;
        let invalid =
            |what: &str, why: &str| CorpusError::Invalid { what: what.to_string(), why: why.to_string() };
        let text = |node: &Node, key: &str| -> Result<String> {
            match node.get(key) {
                Some(Node::Str(s)) => Ok(s.clone()),
                _ => Err(CorpusError::Missing(format!("string field `{key}`"))),
            }
        };
        if text(&root, "schema_version")? != SCHEMA {
            return Err(invalid("schema_version", &format!("only `{SCHEMA}` is understood")));
        }
        let fixture_id = text(&root, "fixture_id")?;
        if !is_fixture_id(&fixture_id) {
            return Err(invalid("fixture_id", "does not match ^[a-z0-9][a-z0-9._-]*$"));
        }
        let class_text = text(&root, "evidence_class")?;
        let evidence_class = match EvidenceClass::parse(&class_text) {
            Some(EvidenceClass::KnownTruthExact) => EvidenceClass::KnownTruthExact,
            _ => {
                return Err(invalid(
                    "evidence_class",
                    "an authored record is KNOWN_TRUTH_EXACT or it is not a record",
                ));
            }
        };
        let units_node = root.get("units").ok_or_else(|| CorpusError::Missing("`units`".to_string()))?;
        let scale = units_node
            .get("scale_to_canonical")
            .and_then(Node::as_scalar)
            .ok_or_else(|| CorpusError::Missing("`units.scale_to_canonical`".to_string()))?
            .clone();
        if let Some(c) = units_node.get("canonical") {
            if c != &Node::Str("metre".to_string()) {
                return Err(invalid("units.canonical", "the canonical unit is the metre"));
            }
        }
        let units = Units { name: text(units_node, "name")?, scale_to_canonical: scale };
        if units.name.is_empty() {
            return Err(invalid("units.name", "is empty"));
        }
        match root.get("provenance") {
            Some(p) if p.get("derivation") == Some(&Node::Str("AUTHORING_PARAMETERS".to_string())) => {}
            _ => return Err(invalid("provenance.derivation", "must be AUTHORING_PARAMETERS")),
        }
        match root.get("assertions").and_then(Node::as_list) {
            Some(a) if !a.is_empty() => {}
            _ => return Err(invalid("assertions", "at least one assertion is required")),
        }
        let list = root
            .get("entities")
            .and_then(Node::as_list)
            .ok_or_else(|| CorpusError::Missing("`entities`".to_string()))?;
        if list.is_empty() {
            return Err(invalid("entities", "a record describes at least one entity"));
        }
        let mut entities = Vec::with_capacity(list.len());
        for (i, node) in list.iter().enumerate() {
            let at = format!("entities[{i}]");
            let id = text(node, "id")?;
            let class_name = text(node, "class")?;
            let class = EntityClass::parse(&class_name).ok_or_else(|| {
                invalid(&format!("{at}.class"), &format!("`{class_name}` is not a curve class of the record"))
            })?;
            let geometry =
                node.get("geometry").ok_or_else(|| CorpusError::Missing(format!("`{at}.geometry`")))?.clone();
            for field in class.required_fields() {
                if geometry.get(field).is_none() {
                    return Err(invalid(
                        &format!("{at}.geometry"),
                        &format!("a {class_name} needs `{field}`"),
                    ));
                }
            }
            entities.push(Entity { id, class, geometry, node: node.clone() });
        }
        Ok(ExpectedMath { fixture_id, evidence_class, units, entities, node: root })
    }
    /// Every scalar in the record, with its path.
    pub fn scalars(&self) -> Vec<(String, &Scalar)> {
        let mut out = Vec::new();
        self.node.scalars("", &mut out);
        out
    }
}
