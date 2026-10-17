//! The policy module: the only place in the repository where a numeric tolerance is written down.
//!
//! A repository-wide check (`crates/pvec-checks/tests/tolerance_literals.rs`) fails the gate if a
//! tolerance-shaped literal shows up in any other Rust file. Everything else derives its decisions
//! from [`Policy`].

/// The three tolerance terms and the conditioning bounds that the rules in
/// `docs/NUMERIC_POLICY.md` are built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Policy {
    /// Absolute floor, in coordinate units. Only matters for values near zero (NP-04).
    pub abs: f64,
    /// Relative term, multiplied by the caller-supplied feature scale (NP-04).
    pub rel: f64,
    /// Number of units in the last place of the largest magnitude involved (NP-04).
    pub ulp: u32,
    /// Smallest allowed weight as a fraction of the largest weight (NP-15).
    pub weight_floor: f64,
    /// Largest allowed ratio between the semi-axes of an ellipse (NP-07).
    pub axis_ratio_max: f64,
}

impl Policy {
    /// The v1 policy. The rationale for each value is in `docs/NUMERIC_POLICY.md`.
    pub const V1: Policy = Policy { abs: 1e-12, rel: 1e-10, ulp: 8, weight_floor: 1e-8, axis_ratio_max: 1e6 };

    /// Returns a copy with every continuous tolerance multiplied by `factor`.
    ///
    /// Only the mutation test calls this, to show that widening the policy by an order of
    /// magnitude turns at least one pathological case red. Production code never builds a policy
    /// other than [`Policy::V1`].
    pub fn widened_for_mutation_test(&self, factor: f64) -> Policy {
        Policy {
            abs: self.abs * factor,
            rel: self.rel * factor,
            ulp: (f64::from(self.ulp) * factor) as u32,
            weight_floor: self.weight_floor * factor,
            axis_ratio_max: self.axis_ratio_max * factor,
        }
    }
}

/// Largest coordinate magnitude accepted anywhere (NP-02): 2^40.
pub const MAX_ABS_COORD: f64 = (1u64 << 40) as f64;
/// Largest magnitude of a linear (non-translation) entry of an affine transform (NP-19): 2^20.
pub const MAX_AFFINE_ENTRY: f64 = (1u64 << 20) as f64;

/// Maximum entities in one file (NP-18): 2^28.
pub const MAX_ENTITIES: usize = 1 << 28;
/// Maximum B-spline or Bezier degree (NP-18).
pub const MAX_DEGREE: usize = 16;
/// Maximum control points on one curve (NP-18): 2^16.
pub const MAX_CONTROL_POINTS: usize = 1 << 16;
/// Maximum knots on one curve (NP-18): control points plus degree plus one at the maxima.
pub const MAX_KNOTS: usize = MAX_CONTROL_POINTS + MAX_DEGREE + 1;
/// Maximum vertices in one polyline (NP-18): 2^20.
pub const MAX_POLYLINE_VERTICES: usize = 1 << 20;
/// Maximum segments in one composite curve (NP-18): 2^16.
pub const MAX_COMPOSITE_SEGMENTS: usize = 1 << 16;
/// Maximum curve nesting depth, counting the outermost curve as 1 (NP-18).
pub const MAX_NESTING_DEPTH: usize = 4;
