//! A JSON reader for the tests, and a validator for the subset of JSON Schema the checked-in schema uses.
//! Test support only: the tool itself writes JSON and never reads it.
#![allow(dead_code)]

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<V>),
    Obj(BTreeMap<String, V>),
}

impl V {
    pub fn get(&self, key: &str) -> &V {
        match self {
            V::Obj(m) => m.get(key).unwrap_or_else(|| panic!("no key {key} in {self:?}")),
            _ => panic!("not an object: {self:?}"),
        }
    }
    pub fn at(&self, path: &str) -> &V {
        path.split('.').fold(self, |v, k| v.get(k))
    }
    pub fn str(&self) -> &str {
        match self {
            V::Str(s) => s,
            _ => panic!("not a string: {self:?}"),
        }
    }
    pub fn num(&self) -> f64 {
        match self {
            V::Num(n) => *n,
            _ => panic!("not a number: {self:?}"),
        }
    }
    pub fn arr(&self) -> &[V] {
        match self {
            V::Arr(a) => a,
            _ => panic!("not an array: {self:?}"),
        }
    }
    pub fn keys(&self) -> Vec<&str> {
        match self {
            V::Obj(m) => m.keys().map(String::as_str).collect(),
            _ => panic!("not an object"),
        }
    }
    /// Every key at any depth.
    pub fn all_keys(&self, out: &mut Vec<String>) {
        match self {
            V::Obj(m) => {
                for (k, v) in m {
                    out.push(k.clone());
                    v.all_keys(out);
                }
            }
            V::Arr(a) => a.iter().for_each(|v| v.all_keys(out)),
            _ => {}
        }
    }
}

pub fn parse(text: &str) -> Result<V, String> {
    let mut p = P { s: text.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing characters at {}", p.i));
    }
    Ok(v)
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected {:?} at {}", c as char, self.i))
        }
    }
    fn value(&mut self) -> Result<V, String> {
        self.ws();
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(V::Obj(m));
                }
                loop {
                    self.ws();
                    let V::Str(k) = self.value()? else { return Err("object key".into()) };
                    self.eat(b':')?;
                    let v = self.value()?;
                    if m.insert(k.clone(), v).is_some() {
                        return Err(format!("duplicate key {k}"));
                    }
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(V::Obj(m));
                        }
                        _ => return Err(format!("bad object at {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(V::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(V::Arr(a));
                        }
                        _ => return Err(format!("bad array at {}", self.i)),
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let mut out = String::new();
                loop {
                    match self.s.get(self.i).copied() {
                        None => return Err("unterminated string".into()),
                        Some(b'"') => {
                            self.i += 1;
                            return Ok(V::Str(out));
                        }
                        Some(b'\\') => {
                            self.i += 1;
                            match self.s.get(self.i).copied() {
                                Some(b'n') => out.push('\n'),
                                Some(b'r') => out.push('\r'),
                                Some(b't') => out.push('\t'),
                                Some(b'"') => out.push('"'),
                                Some(b'\\') => out.push('\\'),
                                Some(b'/') => out.push('/'),
                                Some(b'u') => {
                                    let h = std::str::from_utf8(&self.s[self.i + 1..self.i + 5]).unwrap();
                                    out.push(char::from_u32(u32::from_str_radix(h, 16).unwrap()).unwrap());
                                    self.i += 4;
                                }
                                other => return Err(format!("bad escape {other:?}")),
                            }
                            self.i += 1;
                        }
                        Some(_) => {
                            let rest = std::str::from_utf8(&self.s[self.i..]).map_err(|e| e.to_string())?;
                            let c = rest.chars().next().unwrap();
                            out.push(c);
                            self.i += c.len_utf8();
                        }
                    }
                }
            }
            Some(b't') if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(V::Bool(true))
            }
            Some(b'f') if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(V::Bool(false))
            }
            Some(b'n') if self.s[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(V::Null)
            }
            Some(c) if c.is_ascii_digit() || *c == b'-' => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(self.s[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                t.parse().map(V::Num).map_err(|_| format!("bad number {t}"))
            }
            other => Err(format!("unexpected {other:?} at {}", self.i)),
        }
    }
}

/// Validates `v` against `schema`. The keywords understood are `type`, `required`, `properties`,
/// `additionalProperties` (false only), `items`, `enum`, `const`, `minimum`, `oneOf`, `$ref` into `$defs`
/// and `$defs`. A keyword outside this list is an error, so the schema cannot quietly say more than the
/// validator checks.
pub fn validate(schema_root: &V, v: &V) -> Result<(), String> {
    check(schema_root, schema_root, v, "$")
}

const KNOWN: &[&str] = &[
    "$schema",
    "$id",
    "title",
    "description",
    "type",
    "required",
    "properties",
    "additionalProperties",
    "items",
    "enum",
    "const",
    "minimum",
    "oneOf",
    "anyOf",
    "$ref",
    "$defs",
];

fn check(root: &V, schema: &V, v: &V, at: &str) -> Result<(), String> {
    let V::Obj(s) = schema else {
        return match schema {
            V::Bool(true) => Ok(()),
            _ => Err(format!("{at}: schema is not an object")),
        };
    };
    for k in s.keys() {
        if !KNOWN.contains(&k.as_str()) {
            return Err(format!("the schema uses `{k}`, which the test validator does not check"));
        }
    }
    if let Some(V::Str(r)) = s.get("$ref") {
        let name = r.strip_prefix("#/$defs/").ok_or_else(|| format!("bad $ref {r}"))?;
        return check(root, root.get("$defs").get(name), v, at);
    }
    if let Some(V::Arr(options)) = s.get("oneOf") {
        let ok = options.iter().filter(|o| check(root, o, v, at).is_ok()).count();
        if ok != 1 {
            return Err(format!("{at}: matches {ok} of the oneOf options"));
        }
    }
    if let Some(V::Arr(options)) = s.get("anyOf") {
        if !options.iter().any(|o| check(root, o, v, at).is_ok()) {
            let why: Vec<String> = options.iter().filter_map(|o| check(root, o, v, at).err()).collect();
            return Err(format!("{at}: matches none of the anyOf options: {}", why.join(" | ")));
        }
    }
    if let Some(t) = s.get("type") {
        let names: Vec<&str> = match t {
            V::Str(x) => vec![x.as_str()],
            V::Arr(a) => a.iter().map(V::str).collect(),
            _ => return Err("bad type".into()),
        };
        let have = match v {
            V::Null => "null",
            V::Bool(_) => "boolean",
            V::Num(n) if n.fract() == 0.0 => "integer",
            V::Num(_) => "number",
            V::Str(_) => "string",
            V::Arr(_) => "array",
            V::Obj(_) => "object",
        };
        let fits = names.iter().any(|n| *n == have || (*n == "number" && have == "integer"));
        if !fits {
            return Err(format!("{at}: expected {names:?}, found {have}"));
        }
    }
    if let Some(c) = s.get("const") {
        if c != v {
            return Err(format!("{at}: expected {c:?}, found {v:?}"));
        }
    }
    if let Some(V::Arr(e)) = s.get("enum") {
        if !e.contains(v) {
            return Err(format!("{at}: {v:?} is not one of {e:?}"));
        }
    }
    if let (Some(V::Num(min)), V::Num(n)) = (s.get("minimum"), v) {
        if n < min {
            return Err(format!("{at}: {n} is below {min}"));
        }
    }
    if let V::Obj(m) = v {
        if let Some(V::Arr(req)) = s.get("required") {
            for r in req {
                if !m.contains_key(r.str()) {
                    return Err(format!("{at}: missing required key {}", r.str()));
                }
            }
        }
        let props = match s.get("properties") {
            Some(V::Obj(p)) => Some(p),
            _ => None,
        };
        for (k, child) in m {
            match props.and_then(|p| p.get(k)) {
                Some(sub) => check(root, sub, child, &format!("{at}.{k}"))?,
                None if s.get("additionalProperties") == Some(&V::Bool(false)) => {
                    return Err(format!("{at}: unexpected key {k}"));
                }
                None => {}
            }
        }
    }
    if let (V::Arr(items), Some(sub)) = (v, s.get("items")) {
        for (i, item) in items.iter().enumerate() {
            check(root, sub, item, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}
