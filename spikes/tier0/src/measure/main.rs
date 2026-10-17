//! Measures Tier 0 on the four fixtures. Prints markdown tables for docs/spikes/01-7-tier0.md.

use pvec_fixtures::{Drawing, generate};
use std::time::Instant;
use tier0_spike::preview::Form;
use tier0_spike::tier::{Policy, artifact_bytes, assign, check_identity, decode, encode, generate_only};

/// Screen assumption: the whole drawing extent fits 1920 px across.
const SCREEN_PX: f64 = 1920.0;
/// Error budget in pixels at that fit zoom.
const BUDGET_PX: f64 = 0.5;

const P21: [(&str, u64); 4] = [
    ("small-synthetic", 315_128),
    ("spline-heavy", 89_798_481),
    ("million", 365_814_152),
    ("straddler", 35_306_923),
];

fn kind_name(t: u8) -> &'static str {
    ["line", "circle", "arc", "ellipse", "polyline", "spline", "composite"][t as usize]
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let mut fixtures: Vec<Drawing> = generate::all();
    let reps: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);

    println!("## Coverage, budget {BUDGET_PX} px at fit zoom ({SCREEN_PX} px across the extent)\n");
    println!(
        "| fixture | entities | exact | previewed | neither | exact % | previewed % | neither % | scale (px/unit) | budget (units) |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mut detail = String::new();
    let mut sizes = String::new();
    let mut times = String::new();
    let mut forms = String::new();
    let mut collapse = String::new();
    for d in fixtures.iter_mut() {
        let scale = SCREEN_PX / (d.extent[2] - d.extent[0]);
        let tol = BUDGET_PX / scale;
        let n = d.entities.len();
        let t = assign(d, &Policy::DEFAULT, tol);
        check_identity(d, &t).expect("identity");
        let (e, p, x) = (t.exact.len(), t.previews.len(), t.neither.len());
        assert_eq!(e + p + x, n);
        println!(
            "| {} | {n} | {e} | {p} | {x} | {:.3} | {:.3} | {:.3} | {scale:.5} | {tol:.2} |",
            d.name,
            100.0 * e as f64 / n as f64,
            100.0 * p as f64 / n as f64,
            100.0 * x as f64 / n as f64
        );

        // Neither class, individually (grouped by reason; ids listed, capped at 40 per reason).
        let mut by: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
        for (id, r) in &t.neither {
            by.entry(format!("{r:?}")).or_default().push(*id);
        }
        for (r, ids) in &by {
            let shown: Vec<String> = ids.iter().take(40).map(|i| i.to_string()).collect();
            let kinds: std::collections::BTreeMap<&str, usize> =
                ids.iter().fold(Default::default(), |mut m, i| {
                    *m.entry(kind_name(d.entities[*i as usize].geom.tag())).or_default() += 1;
                    m
                });
            detail.push_str(&format!(
                "| {} | `{r}` | {} | {kinds:?} | {}{} |\n",
                d.name,
                ids.len(),
                shown.join(", "),
                if ids.len() > 40 { ", ..." } else { "" }
            ));
        }

        // Deviation and bytes.
        let art = artifact_bytes(d);
        let stream = encode(d, &t);
        let dec = decode(&stream).expect("decode");
        assert_eq!((dec.exact, dec.previews), (e, p));
        let prev_bytes: usize = t.previews.iter().map(|q| q.bytes()).sum();
        let p21 = P21.iter().find(|(n, _)| *n == d.name).unwrap().1;
        let mean = if p > 0 { t.chosen_dev_sum / p as f64 } else { 0.0 };
        sizes.push_str(&format!(
            "| {} | {:.3e} | {:.4} | {:.2} | {:.4} | {mean:.3e} | {} | {} | {:.2}% | {:.3}% | {} |\n",
            d.name,
            t.chosen_dev_max,
            t.chosen_dev_max * scale,
            1.0 / (t.chosen_dev_max.max(f64::MIN_POSITIVE) * scale),
            mean * scale,
            prev_bytes,
            stream.len(),
            100.0 * stream.len() as f64 / art as f64,
            100.0 * stream.len() as f64 / p21 as f64,
            art
        ));

        // Time to a complete Tier 0. Writer side two ways: the full selection (every candidate form
        // built and verified, which is what this spike needs) and generation only (reference curve,
        // adaptive polyline, quantise, no verification). Reader side: decode of the stream.
        let mut sel_ms = Vec::new();
        let mut gen_ms = Vec::new();
        let mut dec_ms = Vec::new();
        for r in 0..reps {
            if r == 0 {
                let s = Instant::now();
                let t2 = assign(d, &Policy::DEFAULT, tol);
                sel_ms.push(s.elapsed().as_secs_f64() * 1000.0);
                std::hint::black_box(t2.previews.len());
            }
            let s = Instant::now();
            let g = generate_only(d, &Policy::DEFAULT, tol);
            gen_ms.push(s.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(g.len());
            let s = Instant::now();
            let r = decode(&stream).unwrap();
            dec_ms.push(s.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(r.sum);
        }
        let (sm, g, dm) = (median(sel_ms), median(gen_ms), median(dec_ms));
        times.push_str(&format!(
            "| {} | {sm:.0} | {:.0} | {g:.1} | {:.0} | {dm:.2} | {:.0} |\n",
            d.name,
            sm * 1_000_000.0 / n as f64,
            g * 1_000_000.0 / n as f64,
            dm * 1_000_000.0 / n as f64
        ));

        // The collapse policy: sub-budget entities become point stubs.
        let tc = assign(d, &Policy { collapse_subpixel: true, ..Policy::DEFAULT }, tol);
        check_identity(d, &tc).expect("identity (collapse)");
        let sc = encode(d, &tc);
        collapse.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.2}% | {:.3e} | {:.4} |\n",
            d.name,
            tc.exact.len(),
            tc.previews.len(),
            tc.neither.len(),
            sc.len(),
            100.0 * sc.len() as f64 / art as f64,
            tc.chosen_dev_max,
            tc.chosen_dev_max * scale
        ));

        // Per (kind, form) comparison.
        for ((k, f), s) in &t.stats {
            forms.push_str(&format!(
                "| {} | {} | {:?} | {} | {} | {:.1} | {:.3e} | {:.3e} |\n",
                d.name,
                kind_name(*k),
                f,
                s.built,
                s.within,
                s.bytes as f64 / s.built.max(1) as f64,
                s.dev_max,
                s.dev_sum / s.built.max(1) as f64
            ));
        }
        // Which form was chosen.
        let mut chosen: std::collections::BTreeMap<(u8, Form), usize> = Default::default();
        for q in &t.previews {
            *chosen.entry((d.entities[q.replaces as usize].geom.tag(), q.form)).or_default() += 1;
        }
        for ((k, f), c) in chosen {
            forms.push_str(&format!("| {} | {} | chosen: {:?} | {c} | | | | |\n", d.name, kind_name(k), f));
        }
    }
    println!("\n## Neither class, every entity accounted for\n");
    println!("| fixture | reason | count | by kind | ids (first 40) |\n|---|---|---|---|---|");
    print!("{detail}");
    println!("\n## Chosen-preview deviation and Tier 0 byte budget\n");
    println!(
        "| fixture | max sagitta (units) | max sagitta (px at fit) | zoom at which max reaches 1 px | mean (px at fit) | mean (units) | preview bytes | Tier 0 stream bytes | % of packed artifact | % of Part 21 | packed artifact bytes |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    print!("{sizes}");
    println!("\n## Time to a complete Tier 0 (median of {reps}, single thread; full selection run once)\n");
    println!(
        "| fixture | full selection with verification (ms) | ns/entity | generation only (ms) | ns/entity | decode (ms) | ns/entity |\n|---|---|---|---|---|---|---|"
    );
    print!("{times}");
    println!("\n## Collapse policy: entities within the budget become point stubs\n");
    println!(
        "| fixture | exact | previewed | neither | Tier 0 stream bytes | % of packed artifact | max deviation (units) | max deviation (px at fit) |\n|---|---|---|---|---|---|---|---|"
    );
    print!("{collapse}");
    println!("\n## Preview forms compared, per curve kind (all candidates)\n");
    println!(
        "| fixture | kind | form | built | within budget | mean bytes | max dev (units) | mean dev (units) |\n|---|---|---|---|---|---|---|---|"
    );
    print!("{forms}");
}
