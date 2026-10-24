//! A strict JSON reader for the corpus files.
//!
//! Stricter than general JSON on purpose: a number with a fraction or an exponent is refused (the
//! corpus carries every real as a `{decimal, f64_hex}` scalar, so a bare float means a value went
//! through a lossy path), duplicate keys are refused, nesting is bounded and trailing bytes are
//! refused. Object order is preserved.

use crate::error::{CorpusError, Result};

/// A parsed JSON value. Integers are the only numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// An integer that fits `i64`.
    Int(i64),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Value>),
    /// An object, in file order, with unique keys.
    Obj(Vec<(String, Value)>),
}

/// The deepest nesting the reader accepts.
pub const MAX_DEPTH: usize = 64;

/// Parses one document.
pub fn parse(bytes: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| CorpusError::Json { at: e.valid_up_to(), why: "not valid UTF-8".to_string() })?;
    let mut p = Parser { s: text.as_bytes(), text, at: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.at != p.s.len() {
        return Err(p.err("trailing data after the document"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    at: usize,
}

impl Parser<'_> {
    fn err(&self, why: &str) -> CorpusError {
        CorpusError::Json { at: self.at, why: why.to_string() }
    }

    fn ws(&mut self) {
        while matches!(self.s.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn expect(&mut self, b: u8) -> Result<()> {
        if self.peek() == Some(b) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", b as char)))
        }
    }

    fn literal(&mut self, word: &str, v: Value) -> Result<Value> {
        if self.s[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(v)
        } else {
            Err(self.err("not a JSON value"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("not a JSON value")),
        }
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.at;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.at += 1;
        }
        let first = self.peek();
        match first {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.at += 1;
                }
            }
            _ => return Err(self.err("a number needs digits")),
        }
        match self.peek() {
            Some(b'.' | b'e' | b'E') => {
                return Err(self.err("a bare float: reals are carried as {decimal, f64_hex} scalars"));
            }
            Some(b'0'..=b'9') => return Err(self.err("a leading zero")),
            _ => {}
        }
        let digits = &self.text[start..self.at];
        digits
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|_| CorpusError::Json { at: start, why: "an integer outside the i64 range".to_string() })
    }

    fn hex4(&mut self) -> Result<u32> {
        let part = self.text.get(self.at..self.at + 4).ok_or_else(|| self.err("a short \\u escape"))?;
        if !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(self.err("a bad \\u escape"));
        }
        self.at += 4;
        Ok(u32::from_str_radix(part, 16).unwrap_or(0))
    }

    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let Some(b) = self.peek() else { return Err(self.err("an unterminated string")) };
            match b {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.at += 1;
                    let esc = self.peek().ok_or_else(|| self.err("an unterminated escape"))?;
                    self.at += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&hi) {
                                if !self.s[self.at..].starts_with(b"\\u") {
                                    return Err(self.err("a lone high surrogate"));
                                }
                                self.at += 2;
                                let lo = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&lo) {
                                    return Err(self.err("a high surrogate without its pair"));
                                }
                                0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                            } else if (0xdc00..0xe000).contains(&hi) {
                                return Err(self.err("a lone low surrogate"));
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).ok_or_else(|| self.err("not a character"))?);
                        }
                        _ => return Err(self.err("an unknown escape")),
                    }
                }
                0..=0x1f => return Err(self.err("a raw control character in a string")),
                _ => {
                    // The input is valid UTF-8 and `at` is on a character boundary here.
                    let ch = self.text[self.at..].chars().next().ok_or_else(|| self.err("bad string"))?;
                    out.push(ch);
                    self.at += ch.len_utf8();
                }
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value> {
        if depth >= MAX_DEPTH {
            return Err(self.err("nesting deeper than the limit"));
        }
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return Err(self.err("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value> {
        if depth >= MAX_DEPTH {
            return Err(self.err("nesting deeper than the limit"));
        }
        self.expect(b'{')?;
        let mut members: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Obj(members));
        }
        loop {
            self.ws();
            let key_at = self.at;
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                self.at = key_at;
                return Err(self.err(&format!("a duplicate key `{key}`")));
            }
            self.ws();
            self.expect(b':')?;
            self.ws();
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Obj(members));
                }
                _ => return Err(self.err("expected `,` or `}`")),
            }
        }
    }
}

impl Value {
    /// The member of an object, if this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// The integer, if this is one.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// The elements, if this is an array.
    pub fn as_arr(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
    /// The members, if this is an object.
    pub fn as_obj(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Obj(m) => Some(m),
            _ => None,
        }
    }
}
