//! Tier assignment, the Tier 0 stream, and the replacement-identity checks.

use crate::model::{exact_bytes, reference_curve};
use crate::preview::*;
use pvec_fixtures::{Drawing, Entity, Geom, Pt};
use std::collections::{BTreeMap, HashSet};

/// What is cheap enough to ship exact in Tier 0.
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// Ship elliptic arcs exact too (used to prove identity stability across reassignment).
    pub exact_ellipses: bool,
    /// Longest polyline shipped exact.
    pub max_exact_polyline: usize,
    /// Replace an otherwise-exact entity by a point stub when the stub is within the budget.
    pub collapse_subpixel: bool,
}

impl Policy {
    /// The default: lines, circles, arcs and short polylines exact; everything else deferred.
    pub const DEFAULT: Policy =
        Policy { exact_ellipses: false, max_exact_polyline: 64, collapse_subpixel: false };
}

/// Why an entity is in neither class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The P1.3 reference evaluator refuses the geometry under the numeric policy.
    Refused(String),
    /// Every preview form either failed to build or exceeded the error budget.
    NoFormWithinBound,
}

/// Aggregated measurement for one (kind, form) pair.
#[derive(Clone, Debug, Default)]
pub struct FormStats {
    /// Entities for which this form could be built.
    pub built: usize,
    /// Of those, how many met the error budget.
    pub within: usize,
    /// Total bytes over the built previews.
    pub bytes: usize,
    /// Largest measured deviation (world units).
    pub dev_max: f64,
    /// Sum of deviations, for the mean.
    pub dev_sum: f64,
}

/// The result of assigning a drawing to Tier 0.
#[derive(Clone, Debug, Default)]
pub struct Tier0 {
    /// Ids shipped exact.
    pub exact: Vec<u32>,
    /// Previews, one per previewed entity.
    pub previews: Vec<Preview>,
    /// Entities in neither class, with the reason.
    pub neither: Vec<(u32, Reason)>,
    /// Per (kind tag, form) measurement of every candidate.
    pub stats: BTreeMap<(u8, Form), FormStats>,
    /// Largest deviation among the previews actually chosen (world units).
    pub chosen_dev_max: f64,
    /// Preview deviations over chosen previews, for the mean.
    pub chosen_dev_sum: f64,
}

fn is_exact_class(g: &Geom, p: &Policy) -> bool {
    match g {
        Geom::Line { .. } | Geom::Circle { .. } | Geom::Arc { .. } => true,
        Geom::Polyline { pts } => pts.len() <= p.max_exact_polyline,
        Geom::Ellipse { .. } => p.exact_ellipses,
        _ => false,
    }
}

/// Assigns every entity to exactly one of: exact, previewed, neither. `tol` is the preview error
/// budget in world units.
pub fn assign(d: &Drawing, policy: &Policy, tol: f64) -> Tier0 {
    let mut t = Tier0::default();
    for e in &d.entities {
        assign_one(e, policy, tol, &mut t);
    }
    t
}

fn assign_one(e: &Entity, policy: &Policy, tol: f64, t: &mut Tier0) {
    let curve = match reference_curve(&e.geom) {
        Ok(c) => c,
        Err(err) => {
            t.neither.push((e.id, Reason::Refused(format!("{err:?}"))));
            return;
        }
    };
    let exact_class = is_exact_class(&e.geom, policy);
    let b = e.geom.bbox();
    if exact_class && !(policy.collapse_subpixel && (b[2] - b[0]).hypot(b[3] - b[1]) <= 2.0 * tol) {
        t.exact.push(e.id);
        return;
    }
    let kind = e.geom.tag();
    if exact_class {
        // Collapse candidate: a point stub, kept only if it is within the budget.
        let dense = dense_samples(&curve, dense_count(&e.geom));
        let p = quantise(e.id, Form::PointStub, false, &point_stub(&e.geom));
        let dev = deviation(&dense, &p.points(), false);
        if dev <= tol {
            t.chosen_dev_max = t.chosen_dev_max.max(dev);
            t.chosen_dev_sum += dev;
            t.previews.push(p);
        } else {
            t.exact.push(e.id);
        }
        return;
    }
    let dense = dense_samples(&curve, dense_count(&e.geom));
    let mut cands: Vec<Preview> = Vec::new();
    let mut consider = |form: Form, closed: bool, pts: Vec<Pt>, t: &mut Tier0| {
        if pts.is_empty() || pts.len() > MAX_PREVIEW_VERTS {
            return;
        }
        let p = quantise(e.id, form, closed, &pts);
        let dev = deviation(&dense, &p.points(), closed);
        let st = t.stats.entry((kind, form)).or_default();
        st.built += 1;
        st.bytes += p.bytes();
        st.dev_max = st.dev_max.max(dev);
        st.dev_sum += dev;
        if dev <= tol {
            st.within += 1;
            cands.push(p);
        }
    };
    consider(Form::PointStub, false, point_stub(&e.geom), t);
    consider(Form::BboxStub, true, bbox_stub(&e.geom), t);
    if let Some(cp) = control_polygon(&e.geom) {
        consider(Form::ControlPolygon, false, cp, t);
    }
    if let Some(pl) = sagitta_polyline(&curve, tol, init_pieces(&e.geom)) {
        consider(Form::SagittaPolyline, false, pl, t);
    }
    match cands.into_iter().min_by_key(|p| (p.bytes(), p.form.code())) {
        Some(p) => {
            let dev = deviation(&dense, &p.points(), p.closed);
            t.chosen_dev_max = t.chosen_dev_max.max(dev);
            t.chosen_dev_sum += dev;
            t.previews.push(p);
        }
        None => t.neither.push((e.id, Reason::NoFormWithinBound)),
    }
}

/// Ground-truth sample count: at least 256, and 16 per initial piece so a long spline is not
/// under-sampled.
pub fn dense_count(g: &Geom) -> usize {
    256.max(16 * init_pieces(g))
}

/// Writer-side cost only: reference curve, adaptive polyline and quantisation for every deferred
/// entity, with no verification of the result and no form comparison. Returns the previews.
pub fn generate_only(d: &Drawing, policy: &Policy, tol: f64) -> Vec<Preview> {
    let mut out = Vec::new();
    for e in &d.entities {
        if is_exact_class(&e.geom, policy) {
            continue;
        }
        let Ok(c) = reference_curve(&e.geom) else { continue };
        if let Some(pl) = sagitta_polyline(&c, tol, init_pieces(&e.geom)) {
            out.push(quantise(e.id, Form::SagittaPolyline, false, &pl));
        }
    }
    out
}

/// Checks that preview and exact resolve to one identity and that every entity is accounted for
/// exactly once. Returns the first violation.
pub fn check_identity(d: &Drawing, t: &Tier0) -> Result<(), String> {
    let n = d.entities.len();
    let mut seen: HashSet<u32> = HashSet::with_capacity(n);
    let mut put = |id: u32, what: &str| -> Result<(), String> {
        if (id as usize) >= n || d.entities[id as usize].id != id {
            return Err(format!("{what} {id} does not resolve to an exact entity"));
        }
        if !seen.insert(id) {
            return Err(format!("{what} {id} is accounted for twice"));
        }
        Ok(())
    };
    for &id in &t.exact {
        put(id, "exact entity")?;
    }
    for p in &t.previews {
        put(p.replaces, "preview replacing")?;
    }
    for (id, _) in &t.neither {
        put(*id, "unplaceable entity")?;
    }
    if seen.len() != n {
        return Err(format!("{} of {n} entities accounted for", seen.len()));
    }
    Ok(())
}

/// Checks that a preview stays within `tol` of its reference curve (two-sided, measured).
pub fn check_bound(e: &Entity, p: &Preview, tol: f64) -> Result<f64, String> {
    let c = reference_curve(&e.geom).map_err(|x| format!("{x:?}"))?;
    let dev = deviation(&dense_samples(&c, dense_count(&e.geom)), &p.points(), p.closed);
    if dev <= tol { Ok(dev) } else { Err(format!("entity {} preview deviates {dev} > {tol}", e.id)) }
}

/// Total bytes of every entity in the exact packed model.
pub fn artifact_bytes(d: &Drawing) -> usize {
    d.entities.iter().map(|e| exact_bytes(&e.geom)).sum()
}

// ---------------------------------------------------------------- the Tier 0 stream

const MAGIC: &[u8; 4] = b"T0P1";
/// Largest polyline the decoder will accept in an exact record (the NP-18 maximum).
pub const MAX_DECODE_POLYLINE: usize = 1 << 20;

/// Encodes Tier 0: header, exact records, previews.
pub fn encode(d: &Drawing, t: &Tier0) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&(t.exact.len() as u32).to_le_bytes());
    b.extend_from_slice(&(t.previews.len() as u32).to_le_bytes());
    let f = |b: &mut Vec<u8>, xs: &[f64]| xs.iter().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
    for &id in &t.exact {
        let g = &d.entities[id as usize].geom;
        b.extend_from_slice(&id.to_le_bytes());
        b.push(g.tag());
        match g {
            Geom::Line { a, b: e } => f(&mut b, &[a[0], a[1], e[0], e[1]]),
            Geom::Circle { c, r } => f(&mut b, &[c[0], c[1], *r]),
            Geom::Arc { c, r, a0, a1 } => f(&mut b, &[c[0], c[1], *r, *a0, *a1]),
            Geom::Ellipse { c, a, b: bb, rot, t0, t1 } => f(&mut b, &[c[0], c[1], *a, *bb, *rot, *t0, *t1]),
            Geom::Polyline { pts } => {
                b.extend_from_slice(&(pts.len() as u32).to_le_bytes());
                pts.iter().for_each(|p| f(&mut b, p));
            }
            _ => unreachable!("only cheap kinds are shipped exact"),
        }
    }
    for p in &t.previews {
        b.extend_from_slice(&p.replaces.to_le_bytes());
        b.push(p.form.code());
        b.push(p.closed as u8);
        b.extend_from_slice(&(p.verts.len() as u16).to_le_bytes());
        f(&mut b, &p.origin);
        for v in &p.verts {
            b.extend_from_slice(&v[0].to_le_bytes());
            b.extend_from_slice(&v[1].to_le_bytes());
        }
    }
    b
}

/// What a decode saw.
#[derive(Debug, PartialEq)]
pub struct Decoded {
    /// Exact records.
    pub exact: usize,
    /// Previews.
    pub previews: usize,
    /// Sum of every coordinate, to force the data to be touched.
    pub sum: f64,
    /// Ids of previews, in order.
    pub preview_ids: Vec<u32>,
}

struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.0.len() {
            return Err("truncated".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| "u32")?))
    }
    fn f64s(&mut self, n: usize) -> Result<f64, String> {
        let nb = n.checked_mul(8).ok_or("overflow")?;
        Ok(self.take(nb)?.chunks_exact(8).map(|c| f64::from_le_bytes(c.try_into().unwrap())).sum())
    }
}

/// Decodes a Tier 0 stream with every length checked against the bytes that remain before anything
/// is allocated or skipped. Never panics on hostile input.
pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let mut r = Rd(bytes);
    if r.take(4)? != MAGIC {
        return Err("bad magic".into());
    }
    let (ne, np) = (r.u32()? as usize, r.u32()? as usize);
    // The smallest exact record is 5 + 24 bytes and the smallest preview 24; a count that cannot fit
    // in the remaining bytes is refused before the loop starts.
    if ne.checked_mul(29).and_then(|x| x.checked_add(np.checked_mul(24)?)).is_none_or(|x| x > r.0.len()) {
        return Err("counts exceed stream".into());
    }
    let mut sum = 0.0;
    for _ in 0..ne {
        r.u32()?;
        let tag = r.take(1)?[0];
        sum += match tag {
            0 => r.f64s(4)?,
            1 => r.f64s(3)?,
            2 => r.f64s(5)?,
            3 => r.f64s(7)?,
            4 => {
                let n = r.u32()? as usize;
                if n > MAX_DECODE_POLYLINE {
                    return Err("polyline too long".into());
                }
                r.f64s(n.checked_mul(2).ok_or("overflow")?)?
            }
            _ => return Err(format!("unknown exact tag {tag}")),
        };
    }
    let mut ids = Vec::with_capacity(np.min(1 << 16));
    for _ in 0..np {
        ids.push(r.u32()?);
        let head = r.take(2)?;
        if Form::from_code(head[0]).is_none() || head[1] > 1 {
            return Err("bad preview header".into());
        }
        let n = u16::from_le_bytes(r.take(2)?.try_into().map_err(|_| "count")?) as usize;
        if n > MAX_PREVIEW_VERTS {
            return Err("preview too long".into());
        }
        sum += r.f64s(2)?;
        let vb = r.take(n * 8)?;
        sum += vb.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap()) as f64).sum::<f64>();
    }
    if !r.0.is_empty() {
        return Err("trailing bytes".into());
    }
    Ok(Decoded { exact: ne, previews: np, sum, preview_ids: ids })
}
