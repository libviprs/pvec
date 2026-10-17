//! Replays the committed viewport script against every model and prints markdown tables.
//!
//!   measure <small-synthetic|spline-heavy|million|straddler|stress-long>

use pvec_fixtures::{Drawing, generate, viewport_script};
use spatial_index_spike::buckets::Buckets;
use spatial_index_spike::cells::Cells;
use spatial_index_spike::quadtree::Quadtree;
use spatial_index_spike::rtree::RTree;
use spatial_index_spike::*;

fn drawing(name: &str) -> Drawing {
    match name {
        "small-synthetic" => generate::small_synthetic(),
        "spline-heavy" => generate::spline_heavy(),
        "million" => generate::million(),
        "straddler" => generate::straddler(),
        "stress-long" => with_long_polylines(&generate::straddler(), 2000, 200, 7),
        other => panic!("unknown fixture {other}"),
    }
}

fn build_all(items: &Items) -> Vec<(Box<dyn Model>, f64)> {
    fn timed<M: Model + 'static>(f: impl FnOnce() -> M) -> (Box<dyn Model>, f64) {
        let t = cpu_secs();
        let m = f();
        (Box::new(m), (cpu_secs() - t) * 1000.0)
    }
    vec![
        timed(|| Quadtree::build(items, 10)),
        timed(|| RTree::build(items, 512, 64)),
        timed(|| RTree::build(items, 64, 32)),
        timed(|| Cells::build(items, 10, false)),
        timed(|| Cells::build(items, 10, true)),
        timed(|| Buckets::build(items, 7)),
        timed(|| Buckets::build(items, 8)),
    ]
}

fn main() {
    let name = std::env::args().nth(1).expect("fixture name");
    if name == "calibration" {
        // Model bytes per entity by kind against layout B's measured bytes on `million`.
        let want = [
            ("line", 36.7),
            ("circle", 28.7),
            ("arc", 44.7),
            ("ellipse", 60.7),
            ("polyline", 128.7),
            ("spline", 237.9),
            ("composite", 212.3),
        ];
        println!("| fixture | kind | model bytes/entity | layout B on million | difference |");
        println!("|---|---|---|---|---|");
        for f in ["straddler", "million"] {
            let d = drawing(f);
            let (mut sum, mut n) = ([0f64; 7], [0f64; 7]);
            for e in &d.entities {
                sum[e.geom.tag() as usize] += size_model(&e.geom) as f64;
                n[e.geom.tag() as usize] += 1.0;
            }
            for (k, (label, b)) in want.iter().enumerate() {
                let m = sum[k] / n[k];
                println!("| {f} | {label} | {m:.1} | {b} | {:+.1}% |", 100.0 * (m - b) / b);
            }
        }
        return;
    }
    let d = drawing(&name);
    let items = items_of(&d);
    println!("load average: {}", std::fs::read_to_string("/proc/loadavg").unwrap_or_default().trim());
    println!(
        "## {} ({} entities, payload model {} bytes, world side {})\n",
        d.name,
        d.entities.len(),
        items.size.iter().map(|&s| s as u64).sum::<u64>(),
        items.world[2]
    );
    let models = build_all(&items);
    println!("| model | index bytes | index / payload | duplication | build cpu ms |");
    println!("|---|---|---|---|---|");
    let total: u64 = items.size.iter().map(|&s| s as u64).sum();
    for (m, ms) in &models {
        println!(
            "| {} | {} | {:.4} | {:.3} | {:.0} |",
            m.name(),
            m.index_bytes(),
            m.index_bytes() as f64 / total as f64,
            m.duplication(),
            ms
        );
    }
    println!("\nWorst-case units (source-space bounds throughout):\n");
    for (m, _) in &models {
        let w: Vec<String> = m.worst(&items).iter().map(|(k, v)| format!("{k}: {v:.4}")).collect();
        println!("* {}: {}", m.name(), w.join("; "));
    }
    let script = viewport_script();
    println!(
        "\nViewport script, bytes and ranges after merging reads at a 64 KiB gap (and, in brackets, no gap). `amp` is bytes fetched over the payload bytes of the entities that actually intersect; `fp` is the share of fetched entities that do not intersect.\n"
    );
    for (m, _) in &models {
        println!("### {}\n", m.name());
        println!(
            "| step | visible | fetched | fp | ranges (gap 0) | bytes | bytes (gap 0) | amp | query cpu us |"
        );
        println!("|---|---|---|---|---|---|---|---|---|");
        let (mut tot_b, mut tot_r, mut tot_b0, mut tot_r0) = (0u64, 0usize, 0u64, 0usize);
        for s in &script {
            let rect = d.viewport(s);
            let t = cpu_secs();
            let plan = m.query(&items, &rect);
            let us = (cpu_secs() - t) * 1_000_000.0;
            let want = truth(&items, &rect);
            let mut got = plan.fetched.clone();
            got.sort_unstable();
            got.dedup();
            let missing = want.iter().filter(|e| got.binary_search(e).is_err()).count();
            assert_eq!(missing, 0, "{} lost {} visible entities at step {}", m.name(), missing, s.name);
            let (r, b) = merge(&plan.all_reads(), 64 * 1024);
            let (r0, b0) = merge(&plan.all_reads(), 0);
            let want_bytes = payload_bytes(&items, &want).max(1);
            let fp = (got.len() - want.len()) as f64 / got.len().max(1) as f64;
            println!(
                "| {} | {} | {} | {:.3} | {} ({}) | {} | {} | {:.2} | {:.0} |",
                s.name,
                want.len(),
                got.len(),
                fp,
                r,
                r0,
                b,
                b0,
                b as f64 / want_bytes as f64,
                us
            );
            tot_b += b;
            tot_r += r;
            tot_b0 += b0;
            tot_r0 += r0;
        }
        println!("| **script total** | | | | {tot_r} ({tot_r0}) | {tot_b} | {tot_b0} | | |\n");
    }
}
