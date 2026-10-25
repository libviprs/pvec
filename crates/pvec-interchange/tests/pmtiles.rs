//! PMTiles packaging of the display-only MVT tiles (issue P6.7, pvec#24).
//!
//! The reader at the top is written from the PMTiles v3 specification and shares no code with the
//! writer in `src/pmtiles.rs`. The two docker-backed tests at the bottom put the archive in front of
//! programs that are not mine: the go-pmtiles verifier, and an S3-compatible object store answering
//! HTTP range requests.

use pvec_entity::EntityId;
use pvec_geometry::{
    Bezier, Circle, Composite, Curve, Placement2, Point2, Polyline, Transform2, Trimmed, Vector2,
};
use pvec_interchange::FidelityKind;
use pvec_interchange::mvt::*;
use pvec_interchange::pmtiles::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

// ---- an independent PMTiles v3 reader -------------------------------------------------------------

struct Hdr {
    root: (u64, u64),
    meta: (u64, u64),
    leaves: (u64, u64),
    data: (u64, u64),
    addressed: u64,
    entries: u64,
    contents: u64,
    clustered: u8,
    internal_compression: u8,
    tile_compression: u8,
    tile_type: u8,
    min_zoom: u8,
    max_zoom: u8,
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn header(b: &[u8]) -> Hdr {
    assert!(b.len() >= 127, "shorter than the header");
    assert_eq!(&b[0..7], b"PMTiles");
    assert_eq!(b[7], 3, "version");
    Hdr {
        root: (u64_at(b, 8), u64_at(b, 16)),
        meta: (u64_at(b, 24), u64_at(b, 32)),
        leaves: (u64_at(b, 40), u64_at(b, 48)),
        data: (u64_at(b, 56), u64_at(b, 64)),
        addressed: u64_at(b, 72),
        entries: u64_at(b, 80),
        contents: u64_at(b, 88),
        clustered: b[96],
        internal_compression: b[97],
        tile_compression: b[98],
        tile_type: b[99],
        min_zoom: b[100],
        max_zoom: b[101],
    }
}

fn varint(b: &[u8], i: &mut usize) -> u64 {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let byte = b[*i];
        *i += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return v;
        }
        shift += 7;
        assert!(shift < 64);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    id: u64,
    offset: u64,
    length: u64,
    run: u64,
}

fn directory(b: &[u8]) -> Vec<Entry> {
    let mut i = 0;
    let n = varint(b, &mut i) as usize;
    let mut es = vec![Entry { id: 0, offset: 0, length: 0, run: 0 }; n];
    let mut last = 0;
    for e in es.iter_mut() {
        last += varint(b, &mut i);
        e.id = last;
    }
    for e in es.iter_mut() {
        e.run = varint(b, &mut i);
    }
    for e in es.iter_mut() {
        e.length = varint(b, &mut i);
    }
    for k in 0..n {
        let v = varint(b, &mut i);
        es[k].offset = if v == 0 && k > 0 { es[k - 1].offset + es[k - 1].length } else { v - 1 };
    }
    assert_eq!(i, b.len(), "trailing bytes in a directory");
    es
}

fn range(a: &[u8], (off, len): (u64, u64)) -> &[u8] {
    &a[off as usize..(off + len) as usize]
}

/// Looks one tile id up the way a client does: root first, then a leaf if the root points at one.
fn find(a: &[u8], id: u64) -> Option<&[u8]> {
    let h = header(a);
    let mut dir = directory(range(a, h.root));
    for _depth in 0..4 {
        let pos = dir.partition_point(|e| e.id <= id);
        if pos == 0 {
            return None;
        }
        let e = dir[pos - 1];
        if e.run > 0 {
            return (id < e.id + e.run).then(|| range(a, (h.data.0 + e.offset, e.length)));
        }
        dir = directory(range(a, (h.leaves.0 + e.offset, e.length)));
    }
    panic!("directories nested too deep");
}

/// Every (id, bytes) of the archive, walking root and leaves.
fn all_tiles(a: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let h = header(a);
    let mut out = Vec::new();
    let mut stack = vec![directory(range(a, h.root))];
    while let Some(dir) = stack.pop() {
        for e in dir {
            if e.run == 0 {
                stack.push(directory(range(a, (h.leaves.0 + e.offset, e.length))));
            } else {
                for k in 0..e.run {
                    out.push((e.id + k, range(a, (h.data.0 + e.offset, e.length)).to_vec()));
                }
            }
        }
    }
    out.sort();
    out
}

/// The tile id of the PMTiles specification, from its reference algorithm (the Hilbert curve of
/// Wikipedia's `xy2d`, stacked by zoom).
fn ref_tile_id(z: u32, x: u64, y: u64) -> u64 {
    let acc: u64 = (0..z).map(|i| 4u64.pow(i)).sum();
    let n = 1u64 << z;
    let (mut x, mut y, mut d) = (x as i64, y as i64, 0u64);
    let mut s = n / 2;
    while s > 0 {
        let rx = i64::from((x & s as i64) > 0);
        let ry = i64::from((y & s as i64) > 0);
        d += s * s * ((3 * rx) ^ ry) as u64;
        if ry == 0 {
            if rx == 1 {
                x = n as i64 - 1 - x;
                y = n as i64 - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    acc + d
}

// ---- a small JSON reader, for the metadata --------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

impl J {
    fn get(&self, k: &str) -> &J {
        match self {
            J::Obj(m) => m.get(k).unwrap_or_else(|| panic!("no key `{k}` in {self:?}")),
            other => panic!("not an object: {other:?}"),
        }
    }
    fn has(&self, k: &str) -> bool {
        matches!(self, J::Obj(m) if m.contains_key(k))
    }
    fn num(&self) -> f64 {
        match self {
            J::Num(n) => *n,
            o => panic!("not a number: {o:?}"),
        }
    }
    fn str(&self) -> &str {
        match self {
            J::Str(s) => s,
            o => panic!("not a string: {o:?}"),
        }
    }
    fn arr(&self) -> &[J] {
        match self {
            J::Arr(a) => a,
            o => panic!("not an array: {o:?}"),
        }
    }
}

fn json(text: &str) -> J {
    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn eat(&mut self, c: u8) {
            self.ws();
            assert_eq!(self.b[self.i], c, "expected `{}` at {}", c as char, self.i);
            self.i += 1;
        }
        fn string(&mut self) -> String {
            self.eat(b'"');
            let mut out = Vec::new();
            loop {
                let c = self.b[self.i];
                self.i += 1;
                match c {
                    b'"' => return String::from_utf8(out).unwrap(),
                    b'\\' => {
                        let e = self.b[self.i];
                        self.i += 1;
                        out.push(match e {
                            b'n' => b'\n',
                            b't' => b'\t',
                            b'r' => b'\r',
                            b'"' | b'\\' | b'/' => e,
                            other => panic!("escape \\{}", other as char),
                        });
                    }
                    c => out.push(c),
                }
            }
        }
        fn value(&mut self) -> J {
            self.ws();
            match self.b[self.i] {
                b'{' => {
                    self.i += 1;
                    let mut m = BTreeMap::new();
                    self.ws();
                    if self.b[self.i] == b'}' {
                        self.i += 1;
                        return J::Obj(m);
                    }
                    loop {
                        let k = self.string();
                        self.eat(b':');
                        let v = self.value();
                        assert!(m.insert(k.clone(), v).is_none(), "duplicate key {k}");
                        self.ws();
                        match self.b[self.i] {
                            b',' => self.i += 1,
                            b'}' => {
                                self.i += 1;
                                return J::Obj(m);
                            }
                            c => panic!("unexpected `{}`", c as char),
                        }
                    }
                }
                b'[' => {
                    self.i += 1;
                    let mut v = Vec::new();
                    self.ws();
                    if self.b[self.i] == b']' {
                        self.i += 1;
                        return J::Arr(v);
                    }
                    loop {
                        v.push(self.value());
                        self.ws();
                        match self.b[self.i] {
                            b',' => self.i += 1,
                            b']' => {
                                self.i += 1;
                                return J::Arr(v);
                            }
                            c => panic!("unexpected `{}`", c as char),
                        }
                    }
                }
                b'"' => J::Str(self.string()),
                b't' => {
                    self.i += 4;
                    J::Bool(true)
                }
                b'f' => {
                    self.i += 5;
                    J::Bool(false)
                }
                b'n' => {
                    self.i += 4;
                    J::Null
                }
                _ => {
                    let s = self.i;
                    while self.i < self.b.len()
                        && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                    {
                        self.i += 1;
                    }
                    J::Num(std::str::from_utf8(&self.b[s..self.i]).unwrap().parse().unwrap())
                }
            }
        }
    }
    let mut p = P { b: text.as_bytes(), i: 0 };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.b.len(), "trailing text after the JSON value");
    v
}

fn metadata(a: &[u8]) -> J {
    let h = header(a);
    json(std::str::from_utf8(range(a, h.meta)).expect("metadata is UTF-8"))
}

// ---- a minimal MVT scanner: layer names, keys, value types -----------------------------------------

/// (layer name -> (key -> JSON type name)) of one tile, read from the raw protobuf.
fn scan_tile(bytes: &[u8]) -> BTreeMap<String, BTreeMap<String, &'static str>> {
    fn vi(b: &[u8], i: &mut usize) -> u64 {
        varint(b, i)
    }
    fn fields(b: &[u8]) -> Vec<(u64, u64, Vec<u8>)> {
        let mut i = 0;
        let mut out = Vec::new();
        while i < b.len() {
            let k = vi(b, &mut i);
            let (f, w) = (k >> 3, k & 7);
            match w {
                0 => out.push((f, w, vi(b, &mut i).to_le_bytes().to_vec())),
                1 => {
                    out.push((f, w, b[i..i + 8].to_vec()));
                    i += 8;
                }
                2 => {
                    let n = vi(b, &mut i) as usize;
                    out.push((f, w, b[i..i + n].to_vec()));
                    i += n;
                }
                other => panic!("wire type {other}"),
            }
        }
        out
    }
    let mut out = BTreeMap::new();
    for (f, _, layer) in fields(bytes) {
        assert_eq!(f, 3);
        let (mut name, mut keys, mut vals) = (String::new(), Vec::new(), Vec::new());
        let mut used: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut feats = Vec::new();
        for (lf, _, body) in fields(&layer) {
            match lf {
                1 => name = String::from_utf8(body).unwrap(),
                2 => feats.push(body),
                3 => keys.push(String::from_utf8(body).unwrap()),
                4 => {
                    let (vf, _, _) = fields(&body).remove(0);
                    vals.push(match vf {
                        3 | 5 | 6 => "Number",
                        7 => "Boolean",
                        1 | 2 => "String",
                        other => panic!("value field {other}"),
                    });
                }
                _ => {}
            }
        }
        for fb in feats {
            for (ff, _, body) in fields(&fb) {
                if ff == 2 {
                    let mut i = 0;
                    while i < body.len() {
                        let k = vi(&body, &mut i) as usize;
                        let v = vi(&body, &mut i) as usize;
                        used.insert((k, v));
                    }
                }
            }
        }
        let m: &mut BTreeMap<String, &'static str> = out.entry(name).or_default();
        for (k, v) in used {
            m.insert(keys[k].clone(), vals[v]);
        }
    }
    out
}

// ---- the named fixture -----------------------------------------------------------------------------

fn eid(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn circle(cx: f64, cy: f64, r: f64) -> Curve {
    Curve::Circle(Circle::new(Placement2::new(p(cx, cy), Vector2::new(1.0, 0.0)).unwrap(), r).unwrap())
}

fn projection() -> Projection {
    let t = Transform2::scale(2.0, 2.0)
        .unwrap()
        .then(&Transform2::rotate(0.4).unwrap())
        .unwrap()
        .then(&Transform2::translate(512.0, 100.0).unwrap())
        .unwrap();
    Projection::new(t, 1024.0, 512.0).unwrap()
}

fn entities(skip_id: Option<u32>) -> Vec<MvtEntity> {
    let arc = Curve::Trimmed(Trimmed::from_bulge(p(20.0, 150.0), p(180.0, 160.0), 0.8).unwrap());
    let bez = Curve::Bezier(
        Bezier::new(vec![p(0.0, 0.0), p(60.0, 190.0), p(140.0, -30.0), p(200.0, 120.0)]).unwrap(),
    );
    let poly = Curve::Polyline(
        Polyline::new(vec![p(0.0, 100.0), p(70.0, 30.0), p(130.0, 170.0), p(200.0, 90.0)]).unwrap(),
    );
    let comp = Curve::Composite(
        Composite::new(vec![
            Curve::Polyline(Polyline::new(vec![p(10.0, 10.0), p(60.0, 20.0)]).unwrap()),
            Curve::Trimmed(Trimmed::from_bulge(p(60.0, 20.0), p(110.0, 70.0), -0.5).unwrap()),
        ])
        .unwrap(),
    );
    let all = vec![
        MvtEntity::new(eid(1), "circles", circle(100.0, 100.0, 80.0)).unwrap(),
        MvtEntity::new(eid(2), "circles", arc).unwrap(),
        MvtEntity::new(eid(3), "splines", bez).unwrap(),
        MvtEntity::new(eid(4), "lines", poly).unwrap(),
        MvtEntity::new(eid(5), "lines", comp).unwrap(),
    ];
    match skip_id {
        None => all,
        Some(id) => all.into_iter().filter(|e| e.id() != eid(id)).collect(),
    }
}

const MAX_Z: u8 = 4;

fn export(ents: &[MvtEntity]) -> Vec<MvtTile> {
    let pr = projection();
    let mut out = Vec::new();
    for z in 0..=MAX_Z {
        for x in 0..(1u32 << z) {
            for y in 0..(1u32 << z) {
                out.push(
                    export_tile(ents, &pr, TileId::new(z, x, y).unwrap(), &MvtOptions::default()).unwrap(),
                );
            }
        }
    }
    out
}

fn digest() -> [u8; 32] {
    let mut d = [0u8; 32];
    for (i, b) in d.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    d
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn prov() -> Provenance {
    Provenance {
        source_digest: Some(digest()),
        tessellation_tolerance_px: Some(0.25),
        tested_zooms: Some((0, MAX_Z)),
        created: None,
    }
}

fn archive() -> (Vec<MvtTile>, Vec<u8>) {
    let tiles = export(&entities(None));
    let a = write_pmtiles(&tiles, &prov(), &PmtilesOptions::default()).expect("the fixture packages");
    (tiles, a)
}

fn nonempty(tiles: &[MvtTile]) -> Vec<&MvtTile> {
    tiles.iter().filter(|t| !t.bytes.is_empty()).collect()
}

// ---- tile ids ----------------------------------------------------------------------------------------

#[test]
fn issue_24_tile_ids_match_the_specification_values() {
    let id = |z, x, y| tile_id(TileId::new(z, x, y).unwrap());
    assert_eq!(id(0, 0, 0), 0);
    assert_eq!(id(1, 0, 0), 1);
    assert_eq!(id(1, 0, 1), 2);
    assert_eq!(id(1, 1, 1), 3);
    assert_eq!(id(1, 1, 0), 4);
    assert_eq!(id(2, 0, 0), 5);
}

#[test]
fn issue_24_tile_ids_agree_with_the_reference_algorithm_and_are_a_bijection_per_zoom() {
    for z in 0..=7u8 {
        let mut seen = BTreeSet::new();
        for x in 0..(1u32 << z) {
            for y in 0..(1u32 << z) {
                let got = tile_id(TileId::new(z, x, y).unwrap());
                assert_eq!(got, ref_tile_id(u32::from(z), u64::from(x), u64::from(y)), "z{z} x{x} y{y}");
                assert!(seen.insert(got));
            }
        }
        assert_eq!(seen.len(), 1usize << (2 * z));
    }
    // and at the top of the supported range, where the zoom offsets are large
    for (x, y) in [(0, 0), (1, 1), (12_345_678, 7_654_321), ((1 << 24) - 1, (1 << 24) - 1)] {
        assert_eq!(tile_id(TileId::new(24, x, y).unwrap()), ref_tile_id(24, u64::from(x), u64::from(y)));
    }
}

// ---- layout ------------------------------------------------------------------------------------------

#[test]
fn issue_24_header_root_directory_and_tiles_read_back() {
    let (tiles, a) = archive();
    let h = header(&a);
    assert_eq!(h.tile_type, 1, "tile type is MVT");
    assert_eq!(h.clustered, 1, "tile data is in tile id order");
    assert_eq!((h.internal_compression, h.tile_compression), (1, 1), "stored uncompressed");
    assert_eq!((h.min_zoom, h.max_zoom), (0, MAX_Z));
    // sections are where the header says and do not overlap
    assert_eq!(h.root.0, 127);
    assert_eq!(h.meta.0, h.root.0 + h.root.1);
    assert_eq!(h.leaves.0, h.meta.0 + h.meta.1);
    assert_eq!(h.data.0, h.leaves.0 + h.leaves.1);
    assert_eq!(a.len() as u64, h.data.0 + h.data.1);
    // the first 16 KiB hold the header and the whole root directory
    assert!(h.root.0 + h.root.1 <= FIRST_READ_BYTES as u64);

    let want: BTreeMap<u64, &[u8]> =
        nonempty(&tiles).iter().map(|t| (tile_id(t.tile), t.bytes.as_slice())).collect();
    assert!(want.len() > 20, "the fixture should fill many tiles");
    assert_eq!(
        (h.addressed, h.entries, h.contents),
        (want.len() as u64, want.len() as u64, want.len() as u64)
    );
    for (id, bytes) in &want {
        assert_eq!(find(&a, *id), Some(*bytes), "tile id {id}");
    }
    let got = all_tiles(&a);
    assert_eq!(got.len(), want.len());
    assert!(got.iter().all(|(id, b)| want.get(id) == Some(&b.as_slice())));
    // an address nobody exported is not there
    assert_eq!(find(&a, u64::MAX / 2), None);
}

#[test]
fn issue_24_empty_tiles_are_not_addressed() {
    let (tiles, a) = archive();
    assert!(tiles.iter().any(|t| t.bytes.is_empty()), "the fixture has empty tiles");
    let ids: BTreeSet<u64> = all_tiles(&a).into_iter().map(|(id, _)| id).collect();
    for t in tiles.iter().filter(|t| t.bytes.is_empty()) {
        assert!(!ids.contains(&tile_id(t.tile)));
    }
}

#[test]
fn issue_24_many_tiles_move_into_leaf_directories_and_the_root_still_fits_in_16_kib() {
    let one = nonempty(&export(&entities(None))).into_iter().find(|t| t.tile.z() == MAX_Z).unwrap().clone();
    let mut tiles = Vec::new();
    for x in 0..200u32 {
        for y in 0..100u32 {
            let mut t = one.clone();
            t.tile = TileId::new(14, x, y).unwrap();
            tiles.push(t);
        }
    }
    let mut pr = prov();
    pr.tested_zooms = Some((0, 14));
    let a = write_pmtiles(&tiles, &pr, &PmtilesOptions::default()).unwrap();
    let h = header(&a);
    assert!(h.leaves.1 > 0, "20000 entries cannot sit in a 16 KiB root");
    assert!(h.root.0 + h.root.1 <= FIRST_READ_BYTES as u64, "root ends at {}", h.root.0 + h.root.1);
    assert_eq!(h.addressed, 20_000);
    for t in tiles.iter().step_by(37) {
        assert_eq!(find(&a, tile_id(t.tile)), Some(one.bytes.as_slice()), "{:?}", t.tile);
    }
    assert_eq!(all_tiles(&a).len(), 20_000);
}

// ---- vector_layers -----------------------------------------------------------------------------------

fn layers_by_id(a: &[u8]) -> BTreeMap<String, J> {
    metadata(a)
        .get("vector_layers")
        .arr()
        .iter()
        .map(|l| (l.get("id").str().to_string(), l.clone()))
        .collect()
}

#[test]
fn issue_24_vector_layers_match_the_layers_actually_emitted() {
    let (tiles, a) = archive();
    // expected, from the tiles by my own scanner: layer -> (fields, min zoom, max zoom)
    let mut want: BTreeMap<String, (BTreeMap<String, &str>, u8, u8)> = BTreeMap::new();
    for t in nonempty(&tiles) {
        for (name, fields) in scan_tile(&t.bytes) {
            let e = want.entry(name).or_insert((BTreeMap::new(), 99, 0));
            e.0.extend(fields);
            e.1 = e.1.min(t.tile.z());
            e.2 = e.2.max(t.tile.z());
        }
    }
    assert_eq!(want.keys().map(String::as_str).collect::<Vec<_>>(), ["circles", "lines", "splines"]);
    let got = layers_by_id(&a);
    assert_eq!(got.keys().collect::<Vec<_>>(), want.keys().collect::<Vec<_>>(), "the layer set differs");
    for (name, (fields, lo, hi)) in &want {
        let l = &got[name];
        let f: BTreeMap<String, String> = match l.get("fields") {
            J::Obj(m) => m.iter().map(|(k, v)| (k.clone(), v.str().to_string())).collect(),
            o => panic!("fields: {o:?}"),
        };
        let w: BTreeMap<String, String> = fields.iter().map(|(k, v)| (k.clone(), v.to_string())).collect();
        assert_eq!(f, w, "fields of {name}");
        assert_eq!(
            (l.get("minzoom").num(), l.get("maxzoom").num()),
            (f64::from(*lo), f64::from(*hi)),
            "{name}"
        );
    }
    assert_eq!(got["circles"].get("fields").get("entity_id").str(), "Number");
    assert_eq!(got["circles"].get("fields").get("display_only").str(), "Boolean");
}

#[test]
fn issue_24_a_layer_that_is_not_emitted_is_not_declared() {
    let tiles = export(&entities(Some(3)));
    let a = write_pmtiles(&tiles, &prov(), &PmtilesOptions::default()).unwrap();
    let got = layers_by_id(&a);
    assert_eq!(got.keys().map(String::as_str).collect::<Vec<_>>(), ["circles", "lines"]);
}

#[test]
fn issue_24_bytes_that_are_not_a_vector_tile_are_refused() {
    let mut tiles = export(&entities(None));
    let i = tiles.iter().position(|t| !t.bytes.is_empty()).unwrap();
    let at = tiles[i].tile;
    tiles[i].bytes = vec![0xff, 0xff, 0xff];
    assert_eq!(
        write_pmtiles(&tiles, &prov(), &PmtilesOptions::default()),
        Err(PmtilesError::BadTileBytes(at))
    );
}

// ---- provenance ---------------------------------------------------------------------------------------

#[test]
fn issue_24_the_archive_states_digest_tolerance_zoom_range_and_fidelity() {
    let (tiles, a) = archive();
    let m = metadata(&a);
    let pv = m.get("pvec");
    assert_eq!(pv.get("source_digest").str(), format!("sha256:{}", hex(&digest())));
    assert_eq!(pv.get("tessellation_tolerance_px").num(), 0.25);
    assert_eq!(pv.get("tested_zoom_min").num(), 0.0);
    assert_eq!(pv.get("tested_zoom_max").num(), f64::from(MAX_Z));
    assert_eq!(pv.get("display_only"), &J::Bool(true));
    assert_eq!(pv.get("canonical"), &J::Bool(false));

    let fid = pv.get("fidelity");
    let mut total = 0usize;
    for kind in FidelityKind::ALL {
        let n: usize = tiles.iter().map(|t| t.ledger.ledger().count(kind)).sum();
        assert_eq!(fid.get(kind.name()).num(), n as f64, "{}", kind.name());
        total += n;
    }
    assert_eq!(fid.get("records").num(), total as f64);
    let max = tiles
        .iter()
        .flat_map(|t| t.ledger.ledger().records())
        .filter_map(|r| r.error_bound())
        .map(|b| b.get())
        .fold(0.0, f64::max);
    assert!(max > 0.0);
    assert_eq!(fid.get("max_error_bound").num(), max);
    // nothing in the summary claims more than display fidelity
    for k in ["Exact", "ExactReparameterized", "EquivalentCompound"] {
        assert_eq!(fid.get(k).num(), 0.0, "{k}");
    }
}

#[test]
fn issue_24_a_missing_provenance_field_fails_the_writer() {
    let tiles = export(&entities(None));
    let w = |p: Provenance| write_pmtiles(&tiles, &p, &PmtilesOptions::default());
    let mut a = prov();
    a.source_digest = None;
    assert_eq!(w(a), Err(PmtilesError::MissingProvenance("source_digest")));
    let mut b = prov();
    b.tessellation_tolerance_px = None;
    assert_eq!(w(b), Err(PmtilesError::MissingProvenance("tessellation_tolerance_px")));
    let mut c = prov();
    c.tested_zooms = None;
    assert_eq!(w(c), Err(PmtilesError::MissingProvenance("tested_zooms")));
    assert_eq!(w(Provenance::default()), Err(PmtilesError::MissingProvenance("source_digest")));
}

#[test]
fn issue_24_nonsense_provenance_values_fail_the_writer() {
    let tiles = export(&entities(None));
    let w = |p: Provenance| write_pmtiles(&tiles, &p, &PmtilesOptions::default());
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut p = prov();
        p.tessellation_tolerance_px = Some(bad);
        assert!(matches!(w(p), Err(PmtilesError::BadProvenance(_))), "{bad}");
    }
    let mut p = prov();
    p.tested_zooms = Some((3, 1));
    assert!(matches!(w(p), Err(PmtilesError::BadProvenance(_))));
}

#[test]
fn issue_24_a_tile_outside_the_tested_zoom_range_is_refused() {
    let tiles = export(&entities(None));
    let mut pr = prov();
    pr.tested_zooms = Some((0, MAX_Z - 1));
    let r = write_pmtiles(&tiles, &pr, &PmtilesOptions::default());
    assert!(matches!(r, Err(PmtilesError::TileOutsideTestedRange(t)) if t.z() == MAX_Z), "{r:?}");
}

#[test]
fn issue_24_the_same_address_twice_is_refused() {
    let mut tiles = export(&entities(None));
    let t = tiles.iter().find(|t| !t.bytes.is_empty()).unwrap().clone();
    let at = t.tile;
    tiles.push(t);
    assert_eq!(
        write_pmtiles(&tiles, &prov(), &PmtilesOptions::default()),
        Err(PmtilesError::DuplicateTile(at))
    );
}

// ---- determinism --------------------------------------------------------------------------------------

#[test]
fn issue_24_packaging_the_same_tiles_twice_is_byte_identical() {
    let tiles = export(&entities(None));
    let a = write_pmtiles(&tiles, &prov(), &PmtilesOptions::default()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let b = write_pmtiles(&export(&entities(None)), &prov(), &PmtilesOptions::default()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn issue_24_input_order_does_not_change_the_archive() {
    let tiles = export(&entities(None));
    let mut rev = tiles.clone();
    rev.reverse();
    let mut shuffled = tiles.clone();
    shuffled.sort_by_key(|t| (t.tile.x() ^ 0x5, t.tile.y().wrapping_mul(31), t.tile.z()));
    let a = write_pmtiles(&tiles, &prov(), &PmtilesOptions::default()).unwrap();
    assert_eq!(a, write_pmtiles(&rev, &prov(), &PmtilesOptions::default()).unwrap());
    assert_eq!(a, write_pmtiles(&shuffled, &prov(), &PmtilesOptions::default()).unwrap());
}

#[test]
fn issue_24_no_timestamp_unless_the_caller_gives_one() {
    let tiles = export(&entities(None));
    let none = write_pmtiles(&tiles, &prov(), &PmtilesOptions::default()).unwrap();
    assert!(!metadata(&none).get("pvec").has("created"));
    let mut pr = prov();
    pr.created = Some("2026-10-24T00:00:00Z".into());
    let some = write_pmtiles(&tiles, &pr, &PmtilesOptions::default()).unwrap();
    assert_eq!(metadata(&some).get("pvec").get("created").str(), "2026-10-24T00:00:00Z");
    // the only difference between the two is the metadata section (and the offsets after it)
    assert_ne!(none, some);
    assert_eq!(all_tiles(&none), all_tiles(&some));
    let hs = header(&some);
    assert!(hs.meta.1 > header(&none).meta.1);
}

// ---- non-canonicality ----------------------------------------------------------------------------------

#[test]
fn issue_24_no_comparison_or_reader_of_the_artifact_accepts_a_pmtiles_archive() {
    let (_, a) = archive();
    // the reference reader the conformance work holds readers to
    let err = pvec_testkit::reference::read(&a).expect_err("the reference reader accepted a PMTiles archive");
    assert!(!format!("{err:?}").is_empty());
    // the container reader's own header parse
    let prefix = &a[..a.len().min(4096)];
    assert!(pvec_container::header::Header::parse(prefix, a.len() as u64).is_err());
}

#[test]
fn issue_24_the_pmtiles_module_offers_no_import_path() {
    let src = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pmtiles.rs")).unwrap();
    for banned in ["pub fn read_pmtiles", "pub fn import", "pub fn parse_pmtiles", "pub fn open"] {
        assert!(!src.contains(banned), "src/pmtiles.rs must not offer `{banned}`");
    }
    assert!(src.contains("never re-imported") || src.contains("not re-imported"));
}

fn doc() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/INTERCHANGE.md")).unwrap()
}

#[test]
fn issue_24_interchange_chapter_6_records_the_non_canonicality_rule() {
    let d = doc();
    let at = d.find("## 6. PMTiles packaging").expect("chapter 6 heading");
    let ch = &d[at..];
    let ch = &ch[..ch[3..].find("\n## ").map_or(ch.len(), |n| n + 3)];
    for needle in [
        "display fidelity only",
        "not an input to any conformance comparison",
        "not re-imported as geometry",
        "first 16 KiB",
        "source artifact digest",
        "tessellation tolerance",
        "tested zoom range",
        "no timestamp",
        "libviprs/libviprs#1028",
    ] {
        assert!(ch.contains(needle), "chapter 6 should say `{needle}`");
    }
}

// ---- containers ----------------------------------------------------------------------------------------

/// `protomaps/go-pmtiles:v1.31.2`, the multi-platform index. The project's own verifier, which is not mine.
const VERIFIER_IMAGE: &str =
    "protomaps/go-pmtiles@sha256:06574f01f55a78f78f887bc7ebf729a5c093c0d6e17d9876300cfcb0758b59d3";
/// `chrislusf/seaweedfs:4.48`, the multi-platform index. An S3-compatible object store.
const STORE_IMAGE: &str =
    "chrislusf/seaweedfs@sha256:4e61d15fd35994cb1e43e1e553dff106794841fd9a99ade2fc8c8bfce4d7872d";

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pvec-pmtiles-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn start(image: &str, work: &Path, command: &[&str]) -> Option<pvec_hermetic::Container> {
    use pvec_hermetic::{Config, Container, Decision, Policy};
    let cfg = Config { image: image.to_string(), mount: work.to_path_buf(), search_path: None };
    match Container::start_with_command(&cfg, command) {
        Ok(c) => Some(c),
        Err(e) => match Policy::from_env().decide(e) {
            Decision::Skip(why) => {
                eprintln!("skipped: {why}");
                None
            }
            Decision::Fail(e) => panic!("the container could not start: {e}"),
        },
    }
}

fn gone(name: &str) -> bool {
    let ps = std::process::Command::new("docker")
        .args(["ps", "-a", "--filter", &format!("name={name}"), "--format", "{{.Names}}"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&ps.stdout).trim().is_empty()
}

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn issue_24_the_go_pmtiles_verifier_accepts_the_archive_and_refuses_a_damaged_one() {
    let work = scratch("verify");
    let (tiles, a) = archive();
    fs::write(work.join("good.pmtiles"), &a).unwrap();
    // a damaged copy: the root directory's length field says it is larger than the file
    let mut bad = a.clone();
    bad[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    fs::write(work.join("bad.pmtiles"), &bad).unwrap();
    // a copy cut off in the middle of the tile data
    fs::write(work.join("cut.pmtiles"), &a[..a.len() - 40]).unwrap();

    // `serve` keeps the container alive; the checks run with exec.
    let Some(c) = start(VERIFIER_IMAGE, &work, &["serve", "/corpus"]) else {
        let _ = fs::remove_dir_all(&work);
        return;
    };
    let name = c.name().to_string();
    let good = c.exec(&["/go-pmtiles", "verify", "/corpus/good.pmtiles"]).unwrap();
    assert!(good.success, "go-pmtiles verify refused the archive:\n{}\n{}", good.stdout, good.stderr);
    let show = c.exec(&["/go-pmtiles", "show", "/corpus/good.pmtiles"]).unwrap();
    assert!(show.success, "{}{}", show.stdout, show.stderr);
    assert!(
        show.stdout.contains(&format!("addressed tiles count: {}", nonempty(&tiles).len())),
        "{}",
        show.stdout
    );
    for f in ["bad", "cut"] {
        let r = c.exec(&["/go-pmtiles", "verify", &format!("/corpus/{f}.pmtiles")]).unwrap();
        assert!(!r.success, "the verifier accepted {f}.pmtiles:\n{}{}", r.stdout, r.stderr);
    }
    c.stop().unwrap();
    assert!(gone(&name), "the container is still there");
    let _ = fs::remove_dir_all(&work);
}

fn hex_to_bytes(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits.chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
}

#[test]
#[ignore = "needs a docker daemon: the gate-fleet docker-tests stage runs it with --include-ignored"]
fn issue_24_http_range_reads_of_the_archive_in_an_object_store_return_the_right_tiles() {
    let work = scratch("store");
    let (tiles, a) = archive();
    fs::write(work.join("view.pmtiles"), &a).unwrap();

    let Some(c) = start(STORE_IMAGE, &work, &["mini", "-dir=/data"]) else {
        let _ = fs::remove_dir_all(&work);
        return;
    };
    let name = c.name().to_string();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // wait for the S3 gateway, make the bucket, put the archive in it
        let up = c
            .exec(&[
                "sh",
                "-c",
                "for i in $(seq 1 120); do curl -s -o /dev/null http://127.0.0.1:8333/ && exit 0; sleep 1; done; exit 1",
            ])
            .unwrap();
        assert!(up.success, "the object store did not come up:\n{}{}", up.stdout, up.stderr);
        let put = c
            .exec(&[
                "sh",
                "-c",
                "curl -sf -X PUT http://127.0.0.1:8333/maps && \
                 curl -sf -X PUT -T /corpus/view.pmtiles http://127.0.0.1:8333/maps/view.pmtiles",
            ])
            .unwrap();
        assert!(put.success, "upload failed:\n{}{}", put.stdout, put.stderr);

        // a range read as a client does it: the bytes come back as hex, with the status line first
        let get = |from: u64, len: u64| -> Vec<u8> {
            let cmd = format!(
                "curl -s -D /tmp/h -H 'Range: bytes={}-{}' http://127.0.0.1:8333/maps/view.pmtiles | od -An -v -tx1; \
                 head -1 /tmp/h",
                from,
                from + len - 1
            );
            let r = c.exec(&["sh", "-c", &cmd]).unwrap();
            assert!(r.success, "{}{}", r.stdout, r.stderr);
            let (body, status) = r.stdout.rsplit_once("HTTP/").expect("a status line");
            assert!(status.contains("206"), "bytes {from}+{len} did not come back as 206: HTTP/{status}");
            let bytes = hex_to_bytes(body);
            assert_eq!(bytes.len() as u64, len, "short range read at {from}");
            bytes
        };

        let h = header(&a);
        // 1. the first 16 KiB, as a client's first request: header and root directory
        let first = get(0, FIRST_READ_BYTES.min(a.len()) as u64);
        assert_eq!(first, a[..first.len()]);
        let root = directory(&first[h.root.0 as usize..(h.root.0 + h.root.1) as usize]);
        assert!(!root.is_empty());
        // 2. every tile, by the directory entry the client itself decoded
        let mut checked = 0;
        for t in nonempty(&tiles) {
            let id = tile_id(t.tile);
            let e = root.iter().rfind(|e| e.id <= id).expect("an entry at or below the id");
            assert!(e.run > 0, "the fixture fits in the root, no leaves");
            let got = get(h.data.0 + e.offset, e.length);
            assert_eq!(got, t.bytes, "tile {:?}", t.tile);
            checked += 1;
        }
        assert_eq!(checked, nonempty(&tiles).len());
    }));
    let stopped = c.stop();
    assert!(gone(&name), "the container is still there");
    let _ = fs::remove_dir_all(&work);
    if let Err(p) = result {
        std::panic::resume_unwind(p);
    }
    stopped.unwrap();
}
