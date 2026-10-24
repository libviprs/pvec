//! A small SQLite file reader for the tests, written against the file format document and sharing no
//! code with the writer, so a mistake in one cannot hide a mistake in the other. It reads table
//! b-trees (with overflow chains) and index leaves, and nothing else.
#![allow(dead_code)]

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn int(&self) -> i64 {
        match self {
            Value::Int(v) => *v,
            other => panic!("not an integer: {other:?}"),
        }
    }
    pub fn real(&self) -> f64 {
        match self {
            Value::Real(v) => *v,
            Value::Int(v) => *v as f64,
            other => panic!("not a real: {other:?}"),
        }
    }
    pub fn text(&self) -> &str {
        match self {
            Value::Text(v) => v,
            other => panic!("not text: {other:?}"),
        }
    }
    pub fn blob(&self) -> &[u8] {
        match self {
            Value::Blob(v) => v,
            other => panic!("not a blob: {other:?}"),
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

pub struct Row {
    pub rowid: i64,
    pub values: Vec<Value>,
}

pub struct Db {
    pub bytes: Vec<u8>,
    pub page_size: usize,
}

pub struct MasterRow {
    pub kind: String,
    pub name: String,
    pub table: String,
    pub root: u32,
    pub sql: Option<String>,
}

fn be16(b: &[u8], at: usize) -> usize {
    u16::from_be_bytes([b[at], b[at + 1]]) as usize
}
fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn varint(b: &[u8], at: usize) -> (u64, usize) {
    let mut v: u64 = 0;
    for i in 0..9 {
        let byte = b[at + i];
        if i == 8 {
            return ((v << 8) | byte as u64, 9);
        }
        v = (v << 7) | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            return (v, i + 1);
        }
    }
    unreachable!()
}

pub fn decode_record(rec: &[u8]) -> Vec<Value> {
    let (header_len, n) = varint(rec, 0);
    let mut at = n;
    let mut types = Vec::new();
    while at < header_len as usize {
        let (t, n) = varint(rec, at);
        types.push(t);
        at += n;
    }
    assert_eq!(at, header_len as usize, "record header overran");
    let mut body = header_len as usize;
    let mut out = Vec::new();
    for t in types {
        let (v, len) = match t {
            0 => (Value::Null, 0),
            1..=6 => {
                let len = [0, 1, 2, 3, 4, 6, 8][t as usize];
                let mut v: i64 = if rec[body] & 0x80 != 0 { -1 } else { 0 };
                for i in 0..len {
                    v = (v << 8) | rec[body + i] as i64;
                }
                (Value::Int(v), len)
            }
            7 => {
                let mut a = [0u8; 8];
                a.copy_from_slice(&rec[body..body + 8]);
                (Value::Real(f64::from_be_bytes(a)), 8)
            }
            8 => (Value::Int(0), 0),
            9 => (Value::Int(1), 0),
            t if t >= 12 && t % 2 == 0 => {
                let len = ((t - 12) / 2) as usize;
                (Value::Blob(rec[body..body + len].to_vec()), len)
            }
            t if t >= 13 => {
                let len = ((t - 13) / 2) as usize;
                (Value::Text(String::from_utf8(rec[body..body + len].to_vec()).unwrap()), len)
            }
            t => panic!("serial type {t}"),
        };
        out.push(v);
        body += len;
    }
    assert_eq!(body, rec.len(), "record body length");
    out
}

impl Db {
    pub fn open(bytes: Vec<u8>) -> Db {
        assert!(bytes.len() >= 100, "shorter than the file header");
        assert_eq!(&bytes[..16], b"SQLite format 3\0");
        let raw = be16(&bytes, 16);
        let page_size = if raw == 1 { 65536 } else { raw };
        assert!(page_size.is_power_of_two() && page_size >= 512);
        assert_eq!(bytes.len() % page_size, 0, "the file is not a whole number of pages");
        let pages = be32(&bytes, 28) as usize;
        assert_eq!(pages * page_size, bytes.len(), "the in-header page count disagrees with the length");
        Db { bytes, page_size }
    }

    pub fn page(&self, n: u32) -> &[u8] {
        assert!(n >= 1, "page 0");
        let at = (n as usize - 1) * self.page_size;
        &self.bytes[at..at + self.page_size]
    }

    pub fn header_u32(&self, at: usize) -> u32 {
        be32(&self.bytes, at)
    }

    pub fn page_count(&self) -> u32 {
        (self.bytes.len() / self.page_size) as u32
    }

    fn payload(&self, page: &[u8], cell: usize, local_max: usize, local_min: usize, total: usize) -> Vec<u8> {
        let usable = self.page_size;
        if total <= local_max {
            return page[cell..cell + total].to_vec();
        }
        let k = local_min + (total - local_min) % (usable - 4);
        let local = if k <= local_max { k } else { local_min };
        let mut out = page[cell..cell + local].to_vec();
        let mut next = be32(page, cell + local);
        while out.len() < total {
            assert_ne!(next, 0, "overflow chain ended early");
            let p = self.page(next);
            let take = (total - out.len()).min(usable - 4);
            out.extend_from_slice(&p[4..4 + take]);
            next = be32(p, 0);
        }
        assert_eq!(next, 0, "overflow chain runs past the payload");
        out
    }

    /// Every row of the table b-tree rooted at `root`, in rowid order.
    pub fn table_rows(&self, root: u32) -> Vec<Row> {
        let mut out = Vec::new();
        self.walk_table(root, &mut out);
        out
    }

    fn walk_table(&self, n: u32, out: &mut Vec<Row>) {
        let page = self.page(n);
        let base = if n == 1 { 100 } else { 0 };
        let kind = page[base];
        let cells = be16(page, base + 3);
        match kind {
            0x0d => {
                for i in 0..cells {
                    let cell = be16(page, base + 8 + 2 * i);
                    let (total, a) = varint(page, cell);
                    let (rowid, b) = varint(page, cell + a);
                    let usable = self.page_size;
                    let bytes = self.payload(
                        page,
                        cell + a + b,
                        usable - 35,
                        ((usable - 12) * 32 / 255) - 23,
                        total as usize,
                    );
                    out.push(Row { rowid: rowid as i64, values: decode_record(&bytes) });
                }
            }
            0x05 => {
                for i in 0..cells {
                    let cell = be16(page, base + 12 + 2 * i);
                    self.walk_table(be32(page, cell), out);
                }
                self.walk_table(be32(page, base + 8), out);
            }
            k => panic!("page {n} is not a table b-tree page: type {k:#x}"),
        }
    }

    /// The entries of a single-leaf index b-tree.
    pub fn index_entries(&self, root: u32) -> Vec<Vec<Value>> {
        let page = self.page(root);
        assert_eq!(page[0], 0x0a, "not an index leaf");
        (0..be16(page, 3))
            .map(|i| {
                let cell = be16(page, 8 + 2 * i);
                let (total, a) = varint(page, cell);
                decode_record(&page[cell + a..cell + a + total as usize])
            })
            .collect()
    }

    pub fn master(&self) -> Vec<MasterRow> {
        self.table_rows(1)
            .into_iter()
            .map(|r| MasterRow {
                kind: r.values[0].text().to_string(),
                name: r.values[1].text().to_string(),
                table: r.values[2].text().to_string(),
                root: r.values[3].int() as u32,
                sql: match &r.values[4] {
                    Value::Text(s) => Some(s.clone()),
                    _ => None,
                },
            })
            .collect()
    }

    pub fn find(&self, name: &str) -> MasterRow {
        self.master()
            .into_iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no `{name}` in sqlite_master"))
    }

    pub fn rows(&self, table: &str) -> Vec<Row> {
        let m = self.find(table);
        assert_eq!(m.kind, "table");
        self.table_rows(m.root)
    }

    /// Rows as column name to value, using the column names of the `CREATE TABLE` text (the first
    /// word of each comma separated definition, quotes removed). `INTEGER PRIMARY KEY` columns take the rowid.
    pub fn named_rows(&self, table: &str) -> Vec<BTreeMap<String, Value>> {
        let sql = self.find(table).sql.unwrap();
        let cols = column_names(&sql);
        self.rows(table)
            .into_iter()
            .map(|r| {
                let mut m = BTreeMap::new();
                for (i, c) in cols.iter().enumerate() {
                    let v = r.values.get(i).cloned().unwrap_or(Value::Null);
                    let v = if v.is_null() && sql.contains(&format!("\"{c}\" INTEGER PRIMARY KEY")) {
                        Value::Int(r.rowid)
                    } else {
                        v
                    };
                    m.insert(c.clone(), v);
                }
                m
            })
            .collect()
    }
}

/// Column names of a `CREATE TABLE` statement whose columns are quoted with `"` (the writer's style).
pub fn column_names(sql: &str) -> Vec<String> {
    let open = sql.find('(').unwrap();
    let body = &sql[open + 1..sql.rfind(')').unwrap()];
    let mut depth = 0;
    let mut parts = Vec::new();
    let mut cur = String::new();
    for ch in body.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    parts.push(cur);
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| p.starts_with('"'))
        .map(|p| p[1..].split('"').next().unwrap().to_string())
        .collect()
}

// ---- GeoPackage binary and WKB ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Wkb {
    LineString(Vec<[f64; 2]>),
    CircularString(Vec<[f64; 2]>),
    CompoundCurve(Vec<Wkb>),
}

pub struct GpkgGeom {
    pub srs_id: i32,
    pub envelope: [f64; 4],
    pub wkb: Wkb,
    pub raw_wkb: Vec<u8>,
}

pub fn decode_gpkg(blob: &[u8]) -> GpkgGeom {
    assert_eq!(&blob[..2], b"GP", "magic");
    assert_eq!(blob[2], 0, "version");
    let flags = blob[3];
    assert_eq!(flags & 1, 1, "little endian header");
    assert_eq!(flags & 0x20, 0, "standard geometry, not extended");
    assert_eq!(flags & 0x10, 0, "not marked empty");
    let env_kind = (flags >> 1) & 7;
    assert_eq!(env_kind, 1, "an XY envelope");
    let srs_id = i32::from_le_bytes(blob[4..8].try_into().unwrap());
    let mut envelope = [0.0; 4];
    for (i, e) in envelope.iter_mut().enumerate() {
        *e = f64::from_le_bytes(blob[8 + 8 * i..16 + 8 * i].try_into().unwrap());
    }
    let raw_wkb = blob[40..].to_vec();
    let (wkb, used) = decode_wkb(&raw_wkb);
    assert_eq!(used, raw_wkb.len(), "trailing bytes after the geometry");
    GpkgGeom { srs_id, envelope, wkb, raw_wkb }
}

pub fn decode_wkb(b: &[u8]) -> (Wkb, usize) {
    assert_eq!(b[0], 1, "little endian WKB");
    let ty = u32::from_le_bytes(b[1..5].try_into().unwrap());
    let count = u32::from_le_bytes(b[5..9].try_into().unwrap()) as usize;
    let mut at = 9;
    match ty {
        2 | 8 => {
            let mut pts = Vec::new();
            for _ in 0..count {
                let x = f64::from_le_bytes(b[at..at + 8].try_into().unwrap());
                let y = f64::from_le_bytes(b[at + 8..at + 16].try_into().unwrap());
                pts.push([x, y]);
                at += 16;
            }
            (if ty == 2 { Wkb::LineString(pts) } else { Wkb::CircularString(pts) }, at)
        }
        9 => {
            let mut parts = Vec::new();
            for _ in 0..count {
                let (g, used) = decode_wkb(&b[at..]);
                parts.push(g);
                at += used;
            }
            (Wkb::CompoundCurve(parts), at)
        }
        t => panic!("WKB type {t}"),
    }
}

/// Centre and radius of the circle through three points, or `None` when they are collinear.
pub fn circumcircle(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<([f64; 2], f64)> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d == 0.0 {
        return None;
    }
    let (a2, b2, c2) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1], c[0] * c[0] + c[1] * c[1]);
    let ux = (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d;
    let uy = (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d;
    Some(([ux, uy], (a[0] - ux).hypot(a[1] - uy)))
}
