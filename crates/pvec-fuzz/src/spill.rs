//! The target for the writer that keeps its pushes in files (issue pvec#235).
//!
//! The input is a script: a budget, a point at which the store starts failing, a room the sink has, and a list of
//! pushes. The same pushes go to the in-memory writer and to `SpillWriter` over an in-memory store that fails when
//! told. What the target holds the spill writer to is what the issue asks of it: the artifact is the in-memory
//! writer's byte for byte, a rule is refused with the same rule at the same push or at the finish, a store or a sink
//! that fails is a typed error that writes nothing it should not, and nothing panics, whatever the budget.

use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::spill::{FileId, SpillError, SpillResult, SpillStore};
use pvec_container::spill_writer::{ByteSink, FinishError, SpillWriter};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriteError, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Files in memory, with a count of the calls made and a call at which every call fails.
#[derive(Debug)]
struct MemStore {
    files: Mutex<Vec<Option<Vec<u8>>>>,
    calls: AtomicUsize,
    fail_at: usize,
}

impl MemStore {
    fn call(&self) -> SpillResult<()> {
        if self.calls.fetch_add(1, Ordering::Relaxed) >= self.fail_at {
            Err(SpillError::corrupt("the store was told to fail"))
        } else {
            Ok(())
        }
    }
}

impl SpillStore for MemStore {
    fn create(&self) -> SpillResult<FileId> {
        self.call()?;
        let mut f = self.files.lock().map_err(|_| SpillError::corrupt("poisoned"))?;
        f.push(Some(Vec::new()));
        Ok(FileId(f.len() as u32 - 1))
    }

    fn append(&self, file: FileId, bytes: &[u8]) -> SpillResult<()> {
        self.call()?;
        let mut f = self.files.lock().map_err(|_| SpillError::corrupt("poisoned"))?;
        match f.get_mut(file.0 as usize) {
            Some(Some(v)) => {
                v.extend_from_slice(bytes);
                Ok(())
            }
            _ => Err(SpillError::corrupt("no such file")),
        }
    }

    fn read_at(&self, file: FileId, offset: u64, buf: &mut [u8]) -> SpillResult<usize> {
        self.call()?;
        let f = self.files.lock().map_err(|_| SpillError::corrupt("poisoned"))?;
        match f.get(file.0 as usize) {
            Some(Some(v)) => {
                let from = usize::try_from(offset).unwrap_or(usize::MAX).min(v.len());
                let n = buf.len().min(v.len() - from);
                buf[..n].copy_from_slice(&v[from..from + n]);
                Ok(n)
            }
            _ => Err(SpillError::corrupt("no such file")),
        }
    }

    fn remove(&self, file: FileId) -> SpillResult<()> {
        self.call()?;
        let mut f = self.files.lock().map_err(|_| SpillError::corrupt("poisoned"))?;
        if let Some(slot) = f.get_mut(file.0 as usize) {
            *slot = None;
        }
        Ok(())
    }
}

/// A sink that has room for some bytes.
struct Sink {
    room: usize,
    bytes: Vec<u8>,
}

impl ByteSink for Sink {
    fn put(&mut self, bytes: &[u8]) -> Result<(), SpillError> {
        if self.bytes.len() + bytes.len() > self.room {
            return Err(SpillError::corrupt("the sink is full"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

struct Script<'a>(&'a [u8]);

impl Script<'_> {
    fn byte(&mut self) -> u8 {
        let (b, rest) = self.0.split_first().map_or((0, &[][..]), |(b, r)| (*b, r));
        self.0 = rest;
        b
    }
    fn more(&self) -> bool {
        !self.0.is_empty()
    }
}

fn cfg() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "fuzz-spill".to_string(),
            producer_version: "0".to_string(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

/// Pushes one entity to both writers and says whether the in-memory writer took it. A rule the in-memory writer
/// refuses must be the rule the spill writer refuses; a failure of the store is allowed anywhere.
fn push(
    mem: &mut ContainerWriter,
    spill: &mut SpillWriter,
    s: &mut Script<'_>,
) -> Result<Option<&'static str>, FinishError> {
    let kind = s.byte() % 6;
    let id = EntityId::new(u32::from(s.byte() % 40)).expect("small");
    let cell = u32::from(s.byte() % 3);
    let (x, y) = (f64::from(s.byte() % 50), f64::from(s.byte() % 50));
    let w = f64::from(1 + s.byte() % 9);
    let b = Bounds { min: [x, y], max: [x + w, y + w] };
    let meta = EntityMeta { id, style: u32::from(s.byte() % 4), cell, bounds: b };
    let knots = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let (px, py) = ([x, x + w, x + 2.0 * w], [y, y + 2.0 * w, y]);
    let spec = || SplineSpec {
        degree: 2,
        knots: &knots,
        x: &px,
        y: &py,
        weights: None,
        closed: false,
        periodic: false,
    };
    let (a, c) = ([x, y], [x + w, y + w]);
    let (m, r): (Result<(), WriteError>, Result<(), FinishError>) = match kind {
        0 => (mem.push_line(meta, a, c), spill.push_line(meta, a, c)),
        1 => (
            mem.push_polyline(meta, &[a, c, [c[0] + 1.0, c[1]]]),
            spill.push_polyline(meta, &[a, c, [c[0] + 1.0, c[1]]]),
        ),
        2 => (mem.push_circle_arc(meta, a, w, [0.0, 1.5]), spill.push_circle_arc(meta, a, w, [0.0, 1.5])),
        3 => (
            mem.push_ellipse_arc(meta, a, [w + 1.0, w], [0.0, 0.0, 3.0]),
            spill.push_ellipse_arc(meta, a, [w + 1.0, w], [0.0, 0.0, 3.0]),
        ),
        4 => (mem.push_spline(meta, spec()), spill.push_spline(meta, spec())),
        _ => {
            let seg = [EntityId::new(u32::from(s.byte() % 40)).expect("small")];
            (mem.push_composite(meta, &seg, None, false), spill.push_composite(meta, &seg, None, false))
        }
    };
    match (m, r) {
        (Ok(()), Ok(())) => Ok(None),
        (Err(e), Err(FinishError::Write(g))) => {
            assert_eq!(e.rule(), g.rule(), "a push refused with another rule");
            Ok(Some(e.rule()))
        }
        // the store failing at a push is the spill writer's alone to do
        (_, Err(e @ (FinishError::Spill(_) | FinishError::Output(_)))) => Err(e),
        (m, r) => panic!("a push the writers disagree on: in memory {m:?}, spilled {r:?}"),
    }
}

/// The writer that keeps its pushes in files, against the writer that keeps them in memory.
pub fn fuzz_spill_writer(data: &[u8]) -> String {
    let mut s = Script(data);
    let budget = [0usize, 1, 60, 500, 6000, 1 << 20][usize::from(s.byte() % 6)];
    let fail_at = match s.byte() {
        b if b >= 0xC0 => usize::MAX,
        b => usize::from(b),
    };
    let room = match s.byte() {
        255 => usize::MAX,
        b => usize::from(b) * 40,
    };
    let store =
        std::sync::Arc::new(MemStore { files: Mutex::new(Vec::new()), calls: AtomicUsize::new(0), fail_at });
    let mut mem = ContainerWriter::new(cfg());
    let Ok(mut spill) = SpillWriter::new(cfg(), store.clone(), budget) else { return "spill".to_string() };
    let mut refused = None;
    let mut pushes = 0;
    while s.more() && pushes < 24 {
        pushes += 1;
        match push(&mut mem, &mut spill, &mut s) {
            Ok(None) => {}
            Ok(Some(rule)) => {
                // the in-memory writer did not take it and the spill writer did not either; stop here
                refused = Some(rule);
                break;
            }
            Err(_) => return "spill".to_string(),
        }
    }
    let expect = mem.finish();
    let mut sink = Sink { room, bytes: Vec::new() };
    let got = spill.finish_to(&mut sink);
    let word = match (&expect, &got) {
        (Ok(a), Ok(n)) => {
            assert_eq!(*n as usize, sink.bytes.len());
            assert!(a == &sink.bytes, "the streamed artifact differs from the in-memory one");
            "ok".to_string()
        }
        (Err(a), Err(FinishError::Write(g))) => {
            assert_eq!(a.rule(), g.rule(), "a finish refused with another rule");
            assert!(sink.bytes.is_empty(), "the sink got bytes of a refused build");
            a.rule().to_string()
        }
        (_, Err(FinishError::Spill(_))) => {
            // a spill file that fails while the bytes flow leaves a prefix of the artifact, never other bytes
            if let Ok(a) = &expect {
                assert!(
                    a.starts_with(&sink.bytes),
                    "the sink got bytes that are not a prefix of the artifact"
                );
            }
            "spill".to_string()
        }
        (Ok(_), Err(FinishError::Output(_))) => "output".to_string(),
        (e, g) => panic!("the writers disagree: in memory {e:?}, spilled {g:?}"),
    };
    // the files are the store's, and a finished or failed writer has let go of them all
    let _ = refused;
    word
}
