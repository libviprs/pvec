//! A small, bounded JSON reader for the metadata of a PMTiles archive.
//!
//! The metadata of an archive is hostile input, so the reader refuses what it does not need: nesting is
//! limited, an object with a repeated key is refused (two readers could pick different values), and a
//! number is kept as the text it was written with, so a value is never rounded on the way through. Only
//! the writer's own output is ever produced by [`Json::write`]: keys sorted, no whitespace.

use std::collections::BTreeMap;

/// The deepest nesting accepted.
const MAX_DEPTH: usize = 16;
/// The most values (of any kind) one document may hold.
const MAX_NODES: usize = 100_000;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    /// The number as written.
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
    nodes: usize,
}

impl Json {
    /// Reads one document that is the whole of `text`.
    pub(crate) fn parse(text: &str) -> Result<Json, &'static str> {
        let mut r = Reader { b: text.as_bytes(), i: 0, nodes: 0 };
        let v = r.value(0)?;
        r.ws();
        if r.i != r.b.len() {
            return Err("trailing characters after the document");
        }
        Ok(v)
    }

    /// Writes the value with sorted keys and no whitespace.
    pub(crate) fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(n),
            Json::Str(s) => quote(out, s),
            Json::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(m) => {
                out.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// `s` as a JSON string literal.
pub(crate) fn quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl Reader<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), &'static str> {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err("unexpected character")
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json, &'static str> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            Err("unexpected character")
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, &'static str> {
        if depth > MAX_DEPTH {
            return Err("nested too deeply");
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("too many values");
        }
        self.ws();
        match self.b.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    self.eat(b':')?;
                    let v = self.value(depth + 1)?;
                    if m.insert(k, v).is_some() {
                        return Err("a key appears twice");
                    }
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        _ => return Err("expected `,` or `}`"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        _ => return Err("expected `,` or `]`"),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(c) if *c == b'-' || c.is_ascii_digit() => {
                let start = self.i;
                while matches!(self.b.get(self.i), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                    self.i += 1;
                }
                let text = core::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad number")?;
                if text.parse::<f64>().is_err() {
                    return Err("bad number");
                }
                Ok(Json::Num(text.to_string()))
            }
            _ => Err("unexpected character"),
        }
    }

    fn hex4(&mut self) -> Result<u32, &'static str> {
        let h = self.b.get(self.i..self.i + 4).ok_or("short escape")?;
        self.i += 4;
        let s = core::str::from_utf8(h).map_err(|_| "bad escape")?;
        u32::from_str_radix(s, 16).map_err(|_| "bad escape")
    }

    fn string(&mut self) -> Result<String, &'static str> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            // copy a run of plain bytes at once, so UTF-8 sequences stay whole
            let start = self.i;
            while let Some(c) = self.b.get(self.i) {
                if *c == b'"' || *c == b'\\' || *c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(core::str::from_utf8(&self.b[start..self.i]).map_err(|_| "not UTF-8")?);
            match self.b.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let e = *self.b.get(self.i).ok_or("short escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xd800..0xdc00).contains(&hi) {
                                if self.b.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return Err("lone surrogate");
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&lo) {
                                    return Err("lone surrogate");
                                }
                                0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(c).ok_or("lone surrogate")?);
                        }
                        _ => return Err("bad escape"),
                    }
                }
                _ => return Err("unterminated string"),
            }
        }
    }
}
