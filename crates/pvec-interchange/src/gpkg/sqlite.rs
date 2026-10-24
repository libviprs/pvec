//! A SQLite database file writer: just enough of the file format to emit a GeoPackage.
//!
//! The format is written down at <https://www.sqlite.org/fileformat2.html>, and this is the part of
//! it a fresh, never-modified database needs: the 100 byte header, table b-trees (leaf and interior
//! pages, with overflow chains for a big row), single-page index b-trees, and the `sqlite_master`
//! table that lists them. There is no free list, no pointer map and no WAL. Writing it by hand is what
//! keeps the export free of any SQLite library, and so free of C source and of library discovery on
//! the build machine. Everything is a pure function of the input: no clock, no random numbers, and
//! the page numbers are handed out in the order the trees are built.

use super::WRITE_SETTINGS;

/// One column value.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub(super) fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }
}

/// Why a file could not be laid out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LayoutError {
    /// A single row or index entry that cannot be placed where the format needs it.
    DoesNotFit(&'static str),
}

fn put_varint(out: &mut Vec<u8>, v: u64) {
    if v > 0x00ff_ffff_ffff_ffff {
        // nine bytes: eight groups of seven bits, then a whole byte
        let mut bytes = [0u8; 9];
        bytes[8] = (v & 0xff) as u8;
        let mut rest = v >> 8;
        for i in (0..8).rev() {
            bytes[i] = (rest & 0x7f) as u8 | 0x80;
            rest >>= 7;
        }
        out.extend_from_slice(&bytes);
        return;
    }
    let mut groups = [0u8; 9];
    let mut n = 0;
    let mut rest = v;
    loop {
        groups[n] = (rest & 0x7f) as u8;
        rest >>= 7;
        n += 1;
        if rest == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(groups[i] | if i > 0 { 0x80 } else { 0 });
    }
}

fn varint_len(v: u64) -> usize {
    let mut tmp = Vec::with_capacity(9);
    put_varint(&mut tmp, v);
    tmp.len()
}

fn int_size(v: i64) -> usize {
    if (-128..=127).contains(&v) {
        1
    } else if (-32_768..=32_767).contains(&v) {
        2
    } else if (-8_388_608..=8_388_607).contains(&v) {
        3
    } else if (-2_147_483_648..=2_147_483_647).contains(&v) {
        4
    } else if (-140_737_488_355_328..=140_737_488_355_327).contains(&v) {
        6
    } else {
        8
    }
}

/// A record: the header of serial types, then the column bodies.
pub(super) fn record(values: &[Value]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut body = Vec::new();
    for v in values {
        match v {
            Value::Null => put_varint(&mut types, 0),
            Value::Int(0) => put_varint(&mut types, 8),
            Value::Int(1) => put_varint(&mut types, 9),
            Value::Int(i) => {
                let size = int_size(*i);
                let code = match size {
                    1 => 1,
                    2 => 2,
                    3 => 3,
                    4 => 4,
                    6 => 5,
                    _ => 6,
                };
                put_varint(&mut types, code);
                body.extend_from_slice(&i.to_be_bytes()[8 - size..]);
            }
            Value::Real(r) => {
                put_varint(&mut types, 7);
                body.extend_from_slice(&r.to_be_bytes());
            }
            Value::Text(s) => {
                put_varint(&mut types, 13 + 2 * s.len() as u64);
                body.extend_from_slice(s.as_bytes());
            }
            Value::Blob(b) => {
                put_varint(&mut types, 12 + 2 * b.len() as u64);
                body.extend_from_slice(b);
            }
        }
    }
    // The header length counts its own varint, which is one byte for anything this writer builds
    // (a row has far fewer than 120 columns).
    let mut out = Vec::with_capacity(1 + types.len() + body.len());
    put_varint(&mut out, 1 + types.len() as u64);
    out.extend_from_slice(&types);
    out.extend_from_slice(&body);
    out
}

/// How SQLite orders two column values of one index column under the BINARY collation.
fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Int(_) | Value::Real(_) => 1,
        Value::Text(_) => 2,
        Value::Blob(_) => 3,
    };
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Int(x), Value::Real(y)) => (*x as f64).partial_cmp(y).unwrap_or(Equal),
        (Value::Real(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(Equal),
        (Value::Real(x), Value::Real(y)) => x.partial_cmp(y).unwrap_or(Equal),
        (Value::Text(x), Value::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Value::Blob(x), Value::Blob(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// One `sqlite_master` row.
struct MasterRow {
    kind: &'static str,
    name: String,
    table: String,
    root: u32,
    sql: Option<String>,
}

/// The file under construction.
pub(super) struct Writer {
    page_size: usize,
    /// Page 1 is reserved and built last; `pages[0]` is a placeholder for it.
    pages: Vec<Vec<u8>>,
    master: Vec<MasterRow>,
}

const HEADER: usize = 100;

impl Writer {
    pub(super) fn new() -> Writer {
        let page_size = WRITE_SETTINGS.page_size as usize;
        Writer { page_size, pages: vec![Vec::new()], master: Vec::new() }
    }

    pub(super) fn page_size(&self) -> usize {
        self.page_size
    }

    fn alloc(&mut self, page: Vec<u8>) -> u32 {
        debug_assert_eq!(page.len(), self.page_size);
        self.pages.push(page);
        self.pages.len() as u32
    }

    /// Splits a payload that does not fit locally and returns the cell bytes after the key.
    fn spill(&mut self, payload: &[u8], max_local: usize, min_local: usize, cell: &mut Vec<u8>) {
        let usable = self.page_size;
        if payload.len() <= max_local {
            cell.extend_from_slice(payload);
            return;
        }
        let k = min_local + (payload.len() - min_local) % (usable - 4);
        let local = if k <= max_local { k } else { min_local };
        cell.extend_from_slice(&payload[..local]);
        let first = self.pages.len() as u32 + 1;
        cell.extend_from_slice(&first.to_be_bytes());
        let rest = &payload[local..];
        let chunks: Vec<&[u8]> = rest.chunks(usable - 4).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let mut page = vec![0u8; usable];
            let next = if i + 1 < chunks.len() { first + i as u32 + 1 } else { 0 };
            page[..4].copy_from_slice(&next.to_be_bytes());
            page[4..4 + chunk.len()].copy_from_slice(chunk);
            self.alloc(page);
        }
    }

    fn table_leaf_cell(&mut self, rowid: i64, payload: &[u8]) -> Vec<u8> {
        let usable = self.page_size;
        let mut cell = Vec::new();
        put_varint(&mut cell, payload.len() as u64);
        put_varint(&mut cell, rowid as u64);
        self.spill(payload, usable - 35, ((usable - 12) * 32 / 255) - 23, &mut cell);
        cell
    }

    fn page_with(&self, kind: u8, offset: usize, cells: &[Vec<u8>], right: Option<u32>) -> Vec<u8> {
        let mut page = vec![0u8; self.page_size];
        let header = if right.is_some() { 12 } else { 8 };
        page[offset] = kind;
        page[offset + 3..offset + 5].copy_from_slice(&(cells.len() as u16).to_be_bytes());
        let mut end = self.page_size;
        for (i, cell) in cells.iter().enumerate() {
            end -= cell.len();
            page[end..end + cell.len()].copy_from_slice(cell);
            let at = offset + header + 2 * i;
            page[at..at + 2].copy_from_slice(&(end as u16).to_be_bytes());
        }
        page[offset + 5..offset + 7].copy_from_slice(&(end as u16).to_be_bytes());
        if let Some(r) = right {
            page[offset + 8..offset + 12].copy_from_slice(&r.to_be_bytes());
        }
        page
    }

    fn capacity(&self, offset: usize, header: usize) -> usize {
        self.page_size - offset - header
    }

    /// Writes a table b-tree for `rows` (rowid and record, in rowid order) and returns its root page.
    /// `fixed` is the page number the root has to take (page 1 for `sqlite_master`).
    fn table_tree(&mut self, rows: &[(i64, Vec<u8>)], fixed: Option<u32>) -> Result<u32, LayoutError> {
        let root_offset = if fixed == Some(1) { HEADER } else { 0 };
        let cells: Vec<(i64, Vec<u8>)> =
            rows.iter().map(|(id, rec)| (*id, self.table_leaf_cell(*id, rec))).collect();
        let place = |w: &mut Writer, page: Vec<u8>| -> u32 {
            match fixed {
                Some(n) => {
                    w.pages[n as usize - 1] = page;
                    n
                }
                None => w.alloc(page),
            }
        };
        let total: usize = cells.iter().map(|(_, c)| c.len() + 2).sum();
        if total <= self.capacity(root_offset, 8) {
            let page = self.page_with(
                0x0d,
                root_offset,
                &cells.iter().map(|c| c.1.clone()).collect::<Vec<_>>(),
                None,
            );
            return Ok(place(self, page));
        }
        // Leaves at offset 0, greedily packed.
        let cap = self.capacity(0, 8);
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut used = 0;
        for (i, (_, c)) in cells.iter().enumerate() {
            if groups.is_empty() || used + c.len() + 2 > cap {
                groups.push(Vec::new());
                used = 0;
            }
            groups.last_mut().unwrap().push(i);
            used += c.len() + 2;
        }
        if groups.len() == 1 {
            // it fits a leaf but not the root's smaller page: split so the root has two children
            let g = groups.pop().unwrap();
            if g.len() < 2 {
                return Err(LayoutError::DoesNotFit("a table row that cannot share page 1"));
            }
            let mid = g.len() / 2;
            groups.push(g[..mid].to_vec());
            groups.push(g[mid..].to_vec());
        }
        let mut level: Vec<(u32, i64)> = Vec::new();
        for g in groups {
            let page =
                self.page_with(0x0d, 0, &g.iter().map(|&i| cells[i].1.clone()).collect::<Vec<_>>(), None);
            let n = self.alloc(page);
            level.push((n, cells[*g.last().unwrap()].0));
        }
        loop {
            let interior = |children: &[(u32, i64)]| -> Vec<Vec<u8>> {
                children[..children.len() - 1]
                    .iter()
                    .map(|(child, key)| {
                        let mut c = child.to_be_bytes().to_vec();
                        put_varint(&mut c, *key as u64);
                        c
                    })
                    .collect()
            };
            let need: usize = interior(&level).iter().map(|c| c.len() + 2).sum();
            if need <= self.capacity(root_offset, 12) {
                let right = level.last().unwrap().0;
                let page = self.page_with(0x05, root_offset, &interior(&level), Some(right));
                return Ok(place(self, page));
            }
            let cap = self.capacity(0, 12);
            let mut chunks: Vec<Vec<(u32, i64)>> = Vec::new();
            let mut used = 0;
            for child in &level {
                let cell = 2 + 4 + varint_len(child.1 as u64);
                if chunks.is_empty() || used + cell > cap {
                    chunks.push(Vec::new());
                    used = 0;
                }
                chunks.last_mut().unwrap().push(*child);
                used += cell;
            }
            if chunks.len() == 1 {
                let c = chunks.pop().unwrap();
                let mid = c.len() / 2;
                chunks.push(c[..mid].to_vec());
                chunks.push(c[mid..].to_vec());
            }
            let mut next = Vec::new();
            for chunk in chunks {
                let right = chunk.last().unwrap().0;
                let key = chunk.last().unwrap().1;
                let page = self.page_with(0x05, 0, &interior(&chunk), Some(right));
                next.push((self.alloc(page), key));
            }
            level = next;
        }
    }

    /// Adds a table with its rows and registers it in `sqlite_master`. Returns the root page.
    pub(super) fn table(
        &mut self,
        name: &str,
        sql: &str,
        rows: &[(i64, Vec<Value>)],
    ) -> Result<u32, LayoutError> {
        let recs: Vec<(i64, Vec<u8>)> = rows.iter().map(|(id, vals)| (*id, record(vals))).collect();
        let root = self.table_tree(&recs, None)?;
        self.master.push(MasterRow {
            kind: "table",
            name: name.into(),
            table: name.into(),
            root,
            sql: Some(sql.into()),
        });
        Ok(root)
    }

    /// Adds the automatic index behind a `PRIMARY KEY` or `UNIQUE` constraint. The entries are the
    /// key columns of each row followed by its rowid; they all have to fit one page.
    pub(super) fn autoindex(
        &mut self,
        table: &str,
        ordinal: usize,
        mut entries: Vec<Vec<Value>>,
    ) -> Result<(), LayoutError> {
        entries.sort_by(|a, b| {
            a.iter()
                .zip(b)
                .map(|(x, y)| compare(x, y))
                .find(|o| o.is_ne())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let cells: Vec<Vec<u8>> = entries
            .iter()
            .map(|e| {
                let rec = record(e);
                let mut c = Vec::new();
                put_varint(&mut c, rec.len() as u64);
                c.extend_from_slice(&rec);
                c
            })
            .collect();
        let total: usize = cells.iter().map(|c| c.len() + 2).sum();
        let usable = self.page_size;
        let max_local = ((usable - 12) * 64 / 255) - 23;
        if total > self.capacity(0, 8) || cells.iter().any(|c| c.len() > max_local) {
            return Err(LayoutError::DoesNotFit("an index that spans more than one page"));
        }
        let page = self.page_with(0x0a, 0, &cells, None);
        let root = self.alloc(page);
        self.master.push(MasterRow {
            kind: "index",
            name: format!("sqlite_autoindex_{table}_{ordinal}"),
            table: table.into(),
            root,
            sql: None,
        });
        Ok(())
    }

    /// Registers a virtual table (an R-tree): its row in `sqlite_master` has root page 0.
    pub(super) fn virtual_table(&mut self, name: &str, sql: &str) {
        self.master.push(MasterRow {
            kind: "table",
            name: name.into(),
            table: name.into(),
            root: 0,
            sql: Some(sql.into()),
        });
    }

    /// Registers a trigger.
    pub(super) fn trigger(&mut self, name: &str, table: &str, sql: &str) {
        self.master.push(MasterRow {
            kind: "trigger",
            name: name.into(),
            table: table.into(),
            root: 0,
            sql: Some(sql.into()),
        });
    }

    /// Lays out `sqlite_master` on page 1, writes the header and returns the file.
    pub(super) fn finish(mut self) -> Result<Vec<u8>, LayoutError> {
        let rows: Vec<(i64, Vec<u8>)> = self
            .master
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let vals = [
                    Value::text(m.kind),
                    Value::text(&m.name),
                    Value::text(&m.table),
                    Value::Int(i64::from(m.root)),
                    m.sql.as_deref().map_or(Value::Null, Value::text),
                ];
                (i as i64 + 1, record(&vals))
            })
            .collect();
        self.table_tree(&rows, Some(1))?;
        let s = WRITE_SETTINGS;
        let n = self.pages.len() as u32;
        let ps = self.page_size;
        let mut out = Vec::with_capacity(ps * self.pages.len());
        for page in &self.pages {
            out.extend_from_slice(page);
        }
        let h = &mut out[..HEADER];
        h[..16].copy_from_slice(b"SQLite format 3\0");
        h[16..18].copy_from_slice(&(ps as u16).to_be_bytes());
        h[18] = 1;
        h[19] = 1;
        h[21] = 64;
        h[22] = 32;
        h[23] = 32;
        h[24..28].copy_from_slice(&1u32.to_be_bytes());
        h[28..32].copy_from_slice(&n.to_be_bytes());
        h[40..44].copy_from_slice(&1u32.to_be_bytes());
        h[44..48].copy_from_slice(&s.schema_format.to_be_bytes());
        h[56..60].copy_from_slice(&s.text_encoding.to_be_bytes());
        h[60..64].copy_from_slice(&s.user_version.to_be_bytes());
        h[68..72].copy_from_slice(&s.application_id.to_be_bytes());
        h[92..96].copy_from_slice(&1u32.to_be_bytes());
        h[96..100].copy_from_slice(&s.sqlite_version_number.to_be_bytes());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_use_the_shortest_form_and_the_nine_byte_form_for_the_top() {
        let enc = |v: u64| {
            let mut o = Vec::new();
            put_varint(&mut o, v);
            o
        };
        assert_eq!(enc(0), [0]);
        assert_eq!(enc(127), [127]);
        assert_eq!(enc(128), [0x81, 0x00]);
        assert_eq!(enc(300), [0x82, 0x2c]);
        assert_eq!(enc(u64::MAX).len(), 9);
        assert_eq!(enc(u64::MAX), [0xff; 9]);
        assert_eq!(enc(1 << 56).len(), 9);
    }

    #[test]
    fn integers_take_the_smallest_serial_type() {
        assert_eq!(record(&[Value::Int(0), Value::Int(1)]), [3, 8, 9]);
        assert_eq!(record(&[Value::Int(-1)]), [2, 1, 0xff]);
        assert_eq!(record(&[Value::Int(300)]), [2, 2, 0x01, 0x2c]);
        assert_eq!(record(&[Value::Int(4294967296)]).len(), 2 + 6);
    }
}
