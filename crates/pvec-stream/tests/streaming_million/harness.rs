//! The harness behind `streaming_million.rs`: the three fixtures, the viewport driver and the
//! load-shed scenario. Stubbed until the red tests are committed.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

pub const SMALL: u32 = 100_000;
pub const MILLION: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mixed,
    SplineHeavy,
    StraddlerHeavy,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Mixed, Kind::SplineHeavy, Kind::StraddlerHeavy];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mixed => "mixed",
            Kind::SplineHeavy => "spline-heavy",
            Kind::StraddlerHeavy => "straddler-heavy",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Wide,
    Mid,
    Tight,
}

impl Scale {
    pub fn name(self) -> &'static str {
        match self {
            Scale::Wide => "wide",
            Scale::Mid => "mid",
            Scale::Tight => "tight",
        }
    }
}

pub struct Counts {
    pub entities: u64,
    pub advanced: u64,
    pub long: u64,
}

pub struct Measured {
    pub directory_ready: Duration,
    pub first_drawing: Duration,
    pub ranges: u64,
    pub bytes: u64,
    pub blocks: u64,
    pub decode_work: u64,
    pub peak_resident: u64,
    pub advanced_set_bytes: u64,
}

impl Measured {
    pub fn resident_without_advanced(&self) -> u64 {
        self.peak_resident - self.advanced_set_bytes
    }
    pub fn line(&self) -> String {
        unimplemented!("P5.8 harness")
    }
}

pub struct Shed {
    pub changes: u32,
    pub jobs_cancelled: u64,
    pub units_planned: u64,
    pub units_started: u64,
    pub units_superseded: u64,
    pub units_discarded: u64,
    pub stale_applied: u64,
    pub final_matches_last_viewport: bool,
}

impl Shed {
    pub fn shed_rate(&self) -> f64 {
        self.units_discarded as f64 / self.units_superseded as f64
    }
    pub fn useful_fraction(&self) -> f64 {
        1.0 - self.units_discarded as f64 / self.units_started as f64
    }
    pub fn line(&self) -> String {
        unimplemented!("P5.8 harness")
    }
}

pub fn generate(_kind: Kind, _entities: u32, _seed: u64) -> Vec<u8> {
    unimplemented!("P5.8 harness")
}

pub fn artifact(_kind: Kind, _entities: u32) -> Arc<[u8]> {
    unimplemented!("P5.8 harness")
}

pub fn counts(_bytes: &[u8]) -> Counts {
    unimplemented!("P5.8 harness")
}

pub fn measure(_kind: Kind, _entities: u32, _scale: Scale) -> Measured {
    unimplemented!("P5.8 harness")
}

pub fn load_shed(_changes: u32, _seconds: u64) -> Shed {
    unimplemented!("P5.8 harness")
}

pub fn results_markdown(_changes: u32, _seconds: u64) -> String {
    unimplemented!("P5.8 harness")
}
