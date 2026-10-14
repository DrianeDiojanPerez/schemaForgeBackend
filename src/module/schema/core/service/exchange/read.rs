use serde_json::{Map, Value};

use crate::package::errdef::Error;

/// A file somebody picked off their disk, so the first thing wrong with it is
/// the thing worth reporting. Every reader here names the path it was looking
/// at, which is what lets the canvas point at the line.
pub(super) fn refuse(path: &str, message: &str) -> Error {
    Error::validation(format!("the file could not be read at `{path}`"))
        .add_violation(path, message)
}

pub(super) fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, Error> {
    value
        .as_object()
        .ok_or_else(|| refuse(path, "expected an object"))
}

pub(super) fn array<'a>(value: &'a Value, path: &str) -> Result<&'a Vec<Value>, Error> {
    value
        .as_array()
        .ok_or_else(|| refuse(path, "expected an array"))
}

pub(super) fn required<'a>(
    parent: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a Value, Error> {
    parent
        .get(key)
        .filter(|value| !value.is_null())
        .ok_or_else(|| refuse(path, "field is required"))
}

pub(super) fn text(value: &Value, path: &str) -> Result<String, Error> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| refuse(path, "expected a string"))
}

pub(super) fn required_text(
    parent: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<String, Error> {
    text(required(parent, key, path)?, path)
}

pub(super) fn optional_text(
    parent: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<String>, Error> {
    match parent.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => text(value, path).map(Some),
    }
}

pub(super) fn flag(
    parent: &Map<String, Value>,
    key: &str,
    path: &str,
    default: bool,
) -> Result<bool, Error> {
    match parent.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| refuse(path, "expected true or false")),
    }
}

pub(super) fn count(
    parent: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<u32>, Error> {
    match parent.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| refuse(path, "expected a whole number that is not negative")),
    }
}

pub(super) fn coordinate(parent: &Map<String, Value>, key: &str, path: &str) -> Result<f64, Error> {
    match parent.get(key) {
        None | Some(Value::Null) => Ok(0.0),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| refuse(path, "expected a number")),
    }
}
