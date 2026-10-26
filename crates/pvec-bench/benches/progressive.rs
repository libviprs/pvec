//! Progressive coverage on the pinned corpus (issue P8.5). Per corpus entry and recorded viewport it measures
//! four stages: (1) the directory ready, (2) a complete Tier 0 drawing, (3) the first refined region and
//! (4) full visible refinement, on three network arms, plus an interaction arm that pans and zooms mid-load.
//!
//! The reads are real: a cold `ContainerReader` over the file for each viewport, with every range it asks for
//! recorded, and the decode work timed with the wall clock. The network is modelled (`completions`): the
//! recorded ranges go through a virtual link with a round trip, a rate and a fixed range concurrency. So the
//! numbers of the network arms are the modelled transfer plus the measured CPU, and the local arm is the CPU.
//!
//! ```text
//! progressive prepare ENTRY OUT                  write ENTRY.tier0.pvec (the corpus entry plus a Tier 0 block per leaf)
//! progressive run ENTRY OUT PLATFORM [REPEATS]   every stage, arm and the interaction arm, as rows on stdout
//! progressive header                             the CSV header
//! progressive report CSV                         the verdict lines the document records
//! ```

use pvec_bench::layout_bench::{HEADER, Row, parse_rows};
use pvec_bench::progressive_bench::{
    ARMS, Arm, PREVIEW_BUDGET_PX, SCREEN_PX, completions, coverage, verdicts,
};
use pvec_bench::shape::Shape;
use pvec_bench::{Spec, viewports};
use pvec_container::source::{ReadAt, ReadAtSource, Recording, SourceError};
use pvec_container::{BlockId, ContainerReader, ReaderConfig};
use pvec_stream::{
    Aabb, GenerationClock, SpatialDirectory, StraddlerIndex, Tier, Tier0Leaves, Tier0Plan, ViewportConfig,
    ViewportQuery, run_job,
};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::hint::black_box;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct FileAt(fs::File);

impl ReadAt for FileAt {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError> {
        self.0.read_at(buf, offset).map_err(|e| SourceError::Io(e.to_string()))
    }
}

/// The three parts of the arm-independent half of one viewport, in the order they are issued.
#[derive(Default)]
struct Sample {
    /// The reads that open the file and the directory, each depending on the one before.
    open: Vec<u64>,
    open_cpu: f64,
    /// The reads that make Tier 0 complete: leaf blocks, straddler pages and payloads, the advanced set.
    tier0: Vec<u64>,
    tier0_cpu: f64,
    /// The refinement reads, one per planned exact request, with the blocks each holds.
    exact: Vec<u64>,
    exact_blocks: Vec<Vec<BlockId>>,
    exact_cpu: f64,
    /// The Tier 0 blocks read, for the interaction arm.
    tier0_blocks: Vec<BlockId>,
    in_view: u64,
    represented: u64,
    previews: u64,
    worst_px: f64,
    anonymous: u64,
}

struct Truth {
    /// The box of each entity and whether it is a spline (which only the advanced set holds).
    boxes: Vec<([f64; 4], bool)>,
    bounds: [f64; 4],
}

fn aabb(w: [f64; 4]) -> Aabb {
    Aabb::new([w[0], w[1]], [w[2], w[3]]).expect("a window")
}

fn scale_of(w: [f64; 4]) -> f64 {
    SCREEN_PX / (w[2] - w[0]).max(w[3] - w[1]).max(0.000_001)
}

fn path(out: &Path, entry: &str) -> PathBuf {
    out.join(format!("{entry}.tier0.pvec"))
}

fn open(file: &Path) -> ContainerReader<Recording<ReadAtSource<FileAt>>> {
    let f = fs::File::open(file).expect("open");
    let len = f.metadata().expect("metadata").len();
    ContainerReader::open(Recording::new(ReadAtSource::new(FileAt(f), len)), ReaderConfig::default())
        .expect("opens")
}

fn lens(reads: &[pvec_container::source::Request]) -> Vec<u64> {
    reads.iter().map(|q| u64::from(q.len)).collect()
}

/// One viewport on a cold reader: the real reads and decode, stage by stage.
fn measure(file: &Path, truth: &Truth, window: [f64; 4]) -> Sample {
    let mut s = Sample::default();
    let t = Instant::now();
    let r = open(file);
    let dir = SpatialDirectory::open(&r).expect("directory").expect("a directory");
    let leaves = Tier0Leaves::new(r.entries(), &dir).expect("tier 0 leaves");
    let long = StraddlerIndex::open(&r).expect("straddler index");
    s.open_cpu = t.elapsed().as_secs_f64();
    s.open = lens(&r.source().requests());
    let mark = s.open.len();

    let (win, scale) = (aabb(window), scale_of(window));
    let t = Instant::now();
    let plan = ViewportQuery::new(win, scale)
        .plan_tiered(&dir, &r, &leaves, &ViewportConfig { prefetch_margin_px: 0.0 })
        .expect("plan");
    let mut t0_ids: Vec<BlockId> = Vec::new();
    let mut exact: Vec<Vec<BlockId>> = Vec::new();
    for q in plan.requests.iter().filter(|q| q.priority == pvec_stream::Priority::Visible) {
        match q.tier {
            Tier::Tier0 => t0_ids.extend(&q.request.blocks),
            Tier::Exact => exact.push(q.request.blocks.clone()),
        }
    }
    let n = truth.boxes.len();
    let mut have = vec![false; n];
    let mut seen_preview = vec![false; n];
    let (mut previews, mut anonymous) = (Vec::<(u32, f64)>::new(), 0u64);
    for block in r.read_blocks(&t0_ids).expect("tier 0 blocks") {
        let leaf = Tier0Plan::decode_leaf(block.as_bytes()).expect("a leaf plan");
        for id in leaf.exact_ids() {
            have[id.index() as usize] = true;
        }
        for p in leaf.previews() {
            let i = p.replaces().index() as usize;
            if i >= n || seen_preview[i] {
                anonymous += 1;
                continue;
            }
            seen_preview[i] = true;
            have[i] = true;
            previews.push((i as u32, p.error_bound()));
        }
    }
    let mut hit = vec![false; n];
    if let Some(ix) = &long {
        let found = ix.query(&r, &win).expect("straddlers");
        let mut payload: Vec<BlockId> = found.iter().map(|h| h.payload.block_id).collect();
        payload.sort();
        payload.dedup();
        for h in &found {
            hit[h.id.index() as usize] = true;
        }
        r.read_blocks(&payload).expect("payload blocks");
    }
    let advanced = plan.advanced_in_view;
    if advanced {
        for c in 0..r.advanced_chunk_count().expect("chunks") {
            black_box(r.advanced(c).expect("chunk"));
        }
    }
    s.tier0_cpu = t.elapsed().as_secs_f64();
    let all = lens(&r.source().requests());
    s.tier0 = all[mark..].to_vec();
    let mark = all.len();
    s.tier0_blocks = t0_ids;

    let (mut in_view, mut represented) = (0u64, 0u64);
    let mut flagged = vec![false; n];
    for (i, (b, spline)) in truth.boxes.iter().enumerate() {
        if b[0] <= window[2] && window[0] <= b[2] && b[1] <= window[3] && window[1] <= b[3] {
            in_view += 1;
            flagged[i] = true;
            if have[i] || hit[i] || (*spline && advanced) {
                represented += 1;
            }
        }
    }
    s.in_view = in_view;
    s.represented = represented;
    for (i, bound) in &previews {
        if flagged[*i as usize] {
            s.previews += 1;
            s.worst_px = s.worst_px.max(bound * scale);
        }
    }
    s.anonymous = anonymous;

    let t = Instant::now();
    let ids: Vec<BlockId> = exact.iter().flatten().copied().collect();
    let mut work = 0usize;
    for block in r.read_blocks(&ids).expect("exact blocks") {
        if let Ok(view) = block.hot() {
            work += view.entity_ids().len();
        }
    }
    black_box(work);
    s.exact_cpu = t.elapsed().as_secs_f64();
    s.exact = lens(&r.source().requests())[mark..].to_vec();
    s.exact_blocks = exact;
    s
}

/// The four stage times of one sample on one arm, and when each exact request is done.
struct Stages {
    directory: f64,
    tier0: f64,
    first: f64,
    full: f64,
    exact_done: Vec<f64>,
}

fn stages(arm: &Arm, s: &Sample) -> Stages {
    let mut at = 0.0;
    for len in &s.open {
        at = completions(arm, at, &[*len])[0];
    }
    let directory = at + s.open_cpu;
    // every later read is issued at once, Tier 0 first, then the refinement behind it
    let sizes: Vec<u64> = s.tier0.iter().chain(&s.exact).copied().collect();
    let done = completions(arm, directory, &sizes);
    let (t0_done, ex_done) = done.split_at(s.tier0.len());
    let net0 = t0_done.iter().copied().fold(directory, f64::max);
    let tier0 = net0 + s.tier0_cpu;
    let total: u64 = s.exact.iter().sum();
    let (first, full) = match ex_done.first() {
        None => (tier0, tier0),
        Some(first_done) => {
            let share = s.exact[0] as f64 / total.max(1) as f64;
            let last = ex_done.iter().copied().fold(0.0, f64::max);
            (first_done.max(tier0) + share * s.exact_cpu, last.max(tier0) + s.exact_cpu)
        }
    };
    Stages { directory, tier0, first, full, exact_done: ex_done.to_vec() }
}

fn p95(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 * 0.95).ceil() as usize).clamp(1, v.len()) - 1]
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

struct Out<'a> {
    entry: &'a str,
    platform: &'a str,
}

impl Out<'_> {
    fn row(&self, table: &str, arm: &str, param: &str, metric: &str, value: f64, note: &str) {
        let r = Row {
            table: table.into(),
            entry: self.entry.into(),
            platform: self.platform.into(),
            arm: arm.into(),
            param: param.into(),
            metric: metric.into(),
            value: Some(value),
            note: note.into(),
        };
        println!("{}", r.line());
    }
}

fn prepare(entry: &str, out: &Path) {
    let spec = Spec::named(entry).unwrap_or_else(|| panic!("{entry}: not a generated corpus entry"));
    let shapes = spec.shapes();
    let (_, all) = pvec_bench::shape::summarise(&shapes);
    let max_error = PREVIEW_BUDGET_PX * (all[2] - all[0]).max(all[3] - all[1]) / SCREEN_PX;
    let producer = format!("{}/{}/{:#x}/tier0", pvec_bench::GENERATOR, spec.name, spec.seed);
    let bytes =
        pvec_bench::shape::artifact(&shapes, spec.unit(), &producer, Default::default(), Some(max_error))
            .unwrap_or_else(|e| panic!("{e}"));
    fs::write(path(out, entry), bytes).expect("write");
}

fn truth_of(entry: &str) -> Truth {
    let shapes = Spec::named(entry).expect("a generated entry").shapes();
    let (_, bounds) = pvec_bench::shape::summarise(&shapes);
    Truth { boxes: shapes.iter().map(|s| (s.bounds(), matches!(s, Shape::Spline(_)))).collect(), bounds }
}

const STAGE_METRICS: [&str; 4] = ["directory_s", "tier0_s", "refine_first_s", "refine_full_s"];

fn run(entry: &str, dir: &Path, platform: &str, repeats: usize) {
    let o = Out { entry, platform };
    let file = path(dir, entry);
    let truth = truth_of(entry);
    let wins = viewports(entry, truth.bounds);
    // per repeat, per arm, the four stage times of every viewport
    let mut times: Vec<Vec<[Vec<f64>; 4]>> = Vec::new();
    let mut last: Vec<Sample> = Vec::new();
    for _ in 0..repeats {
        let samples: Vec<Sample> = wins.iter().map(|w| measure(&file, &truth, *w)).collect();
        times.push(
            ARMS.iter()
                .map(|arm| {
                    let st: Vec<Stages> = samples.iter().map(|s| stages(arm, s)).collect();
                    [
                        st.iter().map(|s| s.directory).collect(),
                        st.iter().map(|s| s.tier0).collect(),
                        st.iter().map(|s| s.first).collect(),
                        st.iter().map(|s| s.full).collect(),
                    ]
                })
                .collect(),
        );
        last = samples;
    }
    let note =
        "network modelled, CPU measured; p95 over the 200 recorded viewports, median of the repeat runs";
    for (a, arm) in ARMS.iter().enumerate() {
        for (m, metric) in STAGE_METRICS.iter().enumerate() {
            let runs: Vec<f64> = times.iter().map(|t| p95(&mut t[a][m].clone())).collect();
            o.row("stage", arm.name, "p95", metric, median(&mut runs.clone()), note);
            for (i, v) in runs.iter().enumerate() {
                o.row("repeat", arm.name, &format!("run{}", i + 1), metric, *v, "p95 of one repeat run");
            }
            let mean = times.iter().map(|t| t[a][m].iter().sum::<f64>() / wins.len() as f64).sum::<f64>()
                / repeats as f64;
            o.row("stage", arm.name, "mean", metric, mean, note);
        }
        let cov: Vec<f64> = last.iter().map(|s| coverage(s.in_view, s.represented)).collect();
        let frac: Vec<f64> = last
            .iter()
            .map(|s| if s.represented == 0 { 0.0 } else { s.previews as f64 / s.represented as f64 })
            .collect();
        o.row(
            "stage",
            arm.name,
            "min",
            "coverage",
            cov.iter().copied().fold(1.0, f64::min),
            "entities represented in viewport over entities intersecting it, worst viewport",
        );
        o.row(
            "stage",
            arm.name,
            "mean",
            "preview_fraction",
            frac.iter().sum::<f64>() / frac.len() as f64,
            "previews over represented entities",
        );
        o.row(
            "stage",
            arm.name,
            "max",
            "worst_preview_px",
            last.iter().map(|s| s.worst_px).fold(0.0, f64::max),
            "the preview error bound times the viewport scale; display fidelity, not geometric exactness",
        );
        o.row(
            "stage",
            arm.name,
            "fit",
            "worst_preview_px",
            last[0].worst_px,
            "the whole-drawing viewport, the zoom the preview budget is set at",
        );
        o.row(
            "stage",
            arm.name,
            "total",
            "previews_without_identity",
            last.iter().map(|s| s.anonymous).sum::<u64>() as f64,
            "previews that replace no entity of the file, or one already replaced",
        );
    }
    let mean = |f: &dyn Fn(&Sample) -> f64| last.iter().map(f).sum::<f64>() / last.len() as f64;
    let cut = "ranges and bytes per viewport, mean over the 200 recorded viewports";
    o.row("stage", "all", "mean", "ranges_directory", mean(&|s| s.open.len() as f64), cut);
    o.row("stage", "all", "mean", "bytes_directory", mean(&|s| s.open.iter().sum::<u64>() as f64), cut);
    o.row("stage", "all", "mean", "ranges_tier0", mean(&|s| s.tier0.len() as f64), cut);
    o.row("stage", "all", "mean", "bytes_tier0", mean(&|s| s.tier0.iter().sum::<u64>() as f64), cut);
    o.row("stage", "all", "mean", "ranges_refine", mean(&|s| s.exact.len() as f64), cut);
    o.row("stage", "all", "mean", "bytes_refine", mean(&|s| s.exact.iter().sum::<u64>() as f64), cut);
    interaction(&o, &file, &truth, &wins);
}

/// A pan and a zoom issued while the refinement of viewport A is still arriving (halfway between the
/// complete Tier 0 and the full refinement, on the 25 Mbit/s arm). The generation clock supersedes A; the
/// exact requests not yet done are cancelled, and what A had decoded that the new view does not need is the
/// discard.
fn interaction(o: &Out, file: &Path, truth: &Truth, wins: &[[f64; 4]]) {
    let arm = &ARMS[1];
    let mut acc: BTreeMap<&str, [f64; 6]> = BTreeMap::new();
    let mut counts: BTreeMap<&str, f64> = BTreeMap::new();
    for window in wins.iter().skip(2).step_by(4) {
        let a = measure(file, truth, *window);
        let st = stages(arm, &a);
        if a.exact.is_empty() {
            continue;
        }
        let at = st.tier0 + 0.5 * (st.full - st.tier0);
        let (w, h) = (window[2] - window[0], window[3] - window[1]);
        let b = truth.bounds;
        let pan = if window[2] + w / 2.0 <= b[2] {
            [window[0] + w / 2.0, window[1], window[2] + w / 2.0, window[3]]
        } else {
            [window[0] - w / 2.0, window[1], window[2] - w / 2.0, window[3]]
        };
        let zoom = [window[0] + w / 4.0, window[1] + h / 4.0, window[2] - w / 4.0, window[3] - h / 4.0];
        for (kind, next) in [("pan", pan), ("zoom", zoom)] {
            let needed: HashSet<BlockId> = {
                let r = open(file);
                let dir = SpatialDirectory::open(&r).expect("directory").expect("a directory");
                let leaves = Tier0Leaves::new(r.entries(), &dir).expect("leaves");
                let plan = ViewportQuery::new(aabb(next), scale_of(next))
                    .plan_tiered(&dir, &r, &leaves, &ViewportConfig { prefetch_margin_px: 0.0 })
                    .expect("plan");
                plan.block_ids().into_iter().collect()
            };
            let r = open(file);
            let bytes_of = |ids: &[BlockId]| -> u64 {
                ids.iter().map(|b| u64::from(r.entries()[b.0 as usize].stored_length)).sum()
            };
            let clock = GenerationClock::new();
            let token = clock.token();
            // the units of A's refinement job, in the order they were requested; the view changes at `at`
            let mut decoded: Vec<BlockId> = a.tier0_blocks.clone();
            let mut cancelled = 0u64;
            let units: Vec<usize> = (0..a.exact_blocks.len()).collect();
            let outcome = run_job(&token, units, |j| -> Result<usize, ()> {
                if st.exact_done[j] > at && clock.current() == token.generation() {
                    clock.advance();
                }
                Ok(j)
            })
            .expect("a job");
            let finished = match outcome {
                pvec_stream::JobOutcome::Complete(done) => done,
                pvec_stream::JobOutcome::Cancelled { done, .. } => done,
            };
            for j in &finished {
                decoded.extend(&a.exact_blocks[*j]);
            }
            for j in finished.len()..a.exact_blocks.len() {
                cancelled += bytes_of(&a.exact_blocks[j]);
            }
            let total = bytes_of(&decoded);
            let wasted: Vec<BlockId> = decoded.iter().copied().filter(|b| !needed.contains(b)).collect();
            let waste = bytes_of(&wasted);
            let e = acc.entry(kind).or_default();
            let share = if total == 0 { 0.0 } else { waste as f64 / total as f64 };
            e[0] += share;
            e[1] = e[1].max(share);
            e[2] += total as f64;
            e[3] += waste as f64;
            e[4] += cancelled as f64;
            e[5] += (a.exact_blocks.len() - finished.len()) as f64;
            *counts.entry(kind).or_default() += 1.0;
        }
    }
    let note = "view changes halfway through the refinement on the 25 Mbit/s arm; the job of the old generation stops at its next checkpoint";
    for (kind, e) in acc {
        let n = counts[kind];
        o.row(
            "interaction",
            arm.name,
            kind,
            "discard_share",
            e[0] / n,
            &format!("{note}; mean over {n} viewports"),
        );
        o.row("interaction", arm.name, kind, "discard_share_max", e[1], note);
        o.row("interaction", arm.name, kind, "bytes_decoded_mean", e[2] / n, note);
        o.row("interaction", arm.name, kind, "bytes_discarded_mean", e[3] / n, note);
        o.row("interaction", arm.name, kind, "bytes_cancelled_mean", e[4] / n, "exact requests not done when the view changed, never decoded; in-flight ones count as cancelled, so an upper bound of what was saved");
        o.row("interaction", arm.name, kind, "requests_cancelled_mean", e[5] / n, note);
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    match a.first().map(String::as_str) {
        Some("header") => println!("{HEADER}"),
        Some("prepare") => prepare(&a[1], Path::new(&a[2])),
        Some("run") => {
            let repeats = a.get(4).map_or(3, |r| r.parse().expect("a repeat count"));
            run(&a[1], Path::new(&a[2]), &a[3], repeats);
        }
        Some("report") => {
            let rows =
                parse_rows(&fs::read_to_string(&a[1]).expect("the CSV")).unwrap_or_else(|e| panic!("{e}"));
            for v in verdicts(&rows) {
                println!("- {} on {}: {} ({})", v.id, v.platform, v.status.label(), v.detail);
            }
        }
        _ => eprintln!("usage: progressive prepare|run ENTRY OUT [PLATFORM [REPEATS]] | header | report CSV"),
    }
}
