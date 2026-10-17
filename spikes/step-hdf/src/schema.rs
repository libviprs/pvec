//! Row types of the entity-table layout (H1), one table per EXPRESS entity type.
#![allow(missing_docs)]

use hdf5_metno::H5Type;
use hdf5_metno::types::VarLenArray;

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Direction {
    pub dx: f64,
    pub dy: f64,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Vector {
    pub dir: u32,
    pub mag: f64,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Placement {
    pub loc: u32,
    pub dir: u32,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct LineRow {
    pub pnt: u32,
    pub vec: u32,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct CircleRow {
    pub pl: u32,
    pub r: f64,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct EllipseRow {
    pub pl: u32,
    pub a: f64,
    pub b: f64,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct TrimRow {
    pub basis_tag: u8,
    pub basis: u32,
    pub t1: f64,
    pub t2: f64,
    pub sense: u8,
}

#[derive(H5Type, Clone, Debug)]
#[repr(C)]
pub struct PolyRow {
    pub pts: VarLenArray<u32>,
}

#[derive(H5Type, Clone, Debug)]
#[repr(C)]
pub struct SplineRow {
    pub degree: u8,
    pub rational: u8,
    pub cps: VarLenArray<u32>,
    pub mults: VarLenArray<u32>,
    pub knots: VarLenArray<f64>,
    pub weights: VarLenArray<f64>,
}

#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct SegRow {
    pub parent_tag: u8,
    pub parent: u32,
    pub same_sense: u8,
    pub transition: u8,
}

#[derive(H5Type, Clone, Debug)]
#[repr(C)]
pub struct CompRow {
    pub segs: VarLenArray<u32>,
}

/// One row of the `GEOMETRIC_CURVE_SET`: which table the entity's top curve lives in.
#[derive(H5Type, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Root {
    pub tag: u8,
    pub row: u32,
}

/// Root and basis tags.
pub mod tag {
    pub const TRIM: u8 = 0;
    pub const CIRCLE: u8 = 1;
    pub const ELLIPSE: u8 = 2;
    pub const POLY: u8 = 3;
    pub const SPLINE: u8 = 4;
    pub const COMP: u8 = 5;
    pub const LINE: u8 = 6;
}
