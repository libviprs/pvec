//! A small independent JSON reader for the report tests (pvec#25). It shares no code with the writer.
#![allow(dead_code)]

use std::collections::BTreeMap;

// ---- a small JSON reader, for the metadata --------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

impl J {
    pub fn get(&self, k: &str) -> &J {
        match self {
            J::Obj(m) => m.get(k).unwrap_or_else(|| panic!("no key `{k}` in {self:?}")),
            other => panic!("not an object: {other:?}"),
        }
    }
    pub fn has(&self, k: &str) -> bool {
        matches!(self, J::Obj(m) if m.contains_key(k))
    }
    pub fn num(&self) -> f64 {
        match self {
            J::Num(n) => *n,
            o => panic!("not a number: {o:?}"),
        }
    }
    pub fn str(&self) -> &str {
        match self {
            J::Str(s) => s,
            o => panic!("not a string: {o:?}"),
        }
    }
    pub fn arr(&self) -> &[J] {
        match self {
            J::Arr(a) => a,
            o => panic!("not an array: {o:?}"),
        }
    }
}

pub fn json(text: &str) -> J {
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

/// Every object key in the document, as dotted paths with array elements folded into `[]`.
pub fn key_paths(v: &J) -> Vec<String> {
    fn walk(v: &J, prefix: &str, out: &mut Vec<String>) {
        match v {
            J::Obj(m) => {
                for (k, c) in m {
                    let p = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    out.push(p.clone());
                    walk(c, &p, out);
                }
            }
            J::Arr(a) => {
                for c in a {
                    walk(c, &format!("{prefix}[]"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, "", &mut out);
    out.sort();
    out.dedup();
    out
}
