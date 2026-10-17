//! The section kinds of a geometry block.

/// One kind of section in a geometry block. A block holds, for each kind present, one count and the
/// columns of that kind, in ascending kind order. The names are the ones `docs/GEOMETRY_ENCODING.md`
/// uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Section {
    /// An unbounded line.
    Line = 0,
    /// A full circle.
    Circle = 1,
    /// A full ellipse.
    Ellipse = 2,
    /// A trimmed curve over a line: a segment.
    TrimmedLine = 3,
    /// A trimmed curve over a circle: a circular arc.
    TrimmedCircle = 4,
    /// A trimmed curve over an ellipse: an elliptical arc.
    TrimmedEllipse = 5,
    /// A polyline.
    Polyline = 6,
    /// A composite curve.
    Composite = 7,
}

impl Section {
    /// Every section kind, in kind order.
    pub const ALL: [Section; 8] = [
        Section::Line,
        Section::Circle,
        Section::Ellipse,
        Section::TrimmedLine,
        Section::TrimmedCircle,
        Section::TrimmedEllipse,
        Section::Polyline,
        Section::Composite,
    ];

    /// The numeric kind id written in the directory.
    pub fn id(self) -> u16 {
        self as u16
    }

    /// The name used in the specification.
    pub fn name(self) -> &'static str {
        match self {
            Section::Line => "Line",
            Section::Circle => "Circle",
            Section::Ellipse => "Ellipse",
            Section::TrimmedLine => "TrimmedLine",
            Section::TrimmedCircle => "TrimmedCircle",
            Section::TrimmedEllipse => "TrimmedEllipse",
            Section::Polyline => "Polyline",
            Section::Composite => "Composite",
        }
    }

    pub(crate) fn from_id(id: u16) -> Option<Section> {
        Section::ALL.iter().copied().find(|s| s.id() == id)
    }

    /// Number of `f64` columns of a fixed-width kind, and whether it carries a flags column.
    pub(crate) fn fixed_shape(self) -> Option<(usize, bool)> {
        match self {
            Section::Line => Some((4, false)),
            Section::Circle => Some((5, false)),
            Section::Ellipse => Some((6, false)),
            Section::TrimmedLine => Some((6, true)),
            Section::TrimmedCircle => Some((7, true)),
            Section::TrimmedEllipse => Some((8, true)),
            _ => None,
        }
    }
}
