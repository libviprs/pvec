//! A strict JSON reader for the corpus files.
//!
//! Stricter than general JSON on purpose: a number with a fraction or an exponent is refused (the
//! corpus carries every real as a `{decimal, f64_hex}` scalar, so a bare float means a value went
//! through a lossy path), duplicate keys are refused, nesting is bounded and trailing bytes are
//! refused. Object order is preserved.

use crate::error::Result;

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
pub fn parse(_bytes: &[u8]) -> Result<Value> {
    todo!("json::parse")
}

impl Value {
    /// The member of an object, if this is an object that has it.
    pub fn get(&self, _key: &str) -> Option<&Value> {
        todo!("Value::get")
    }
    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        todo!("Value::as_str")
    }
    /// The integer, if this is one.
    pub fn as_int(&self) -> Option<i64> {
        todo!("Value::as_int")
    }
    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        todo!("Value::as_bool")
    }
    /// The elements, if this is an array.
    pub fn as_arr(&self) -> Option<&[Value]> {
        todo!("Value::as_arr")
    }
    /// The members, if this is an object.
    pub fn as_obj(&self) -> Option<&[(String, Value)]> {
        todo!("Value::as_obj")
    }
}
