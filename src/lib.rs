// SPDX-License-Identifier: MPL-2.0

mod diagnostic;
mod eval;
mod generated;
mod json;
mod schema;
mod syntax;

use std::fmt;

use indexmap::IndexMap;
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

pub use diagnostic::{Diagnostic, ErrorReport};
pub use schema::Schema;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct OverlayDocument {
    source: Source,
    ast: syntax::OverlayAst,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Object(IndexMap<String, Value>),
    List(Vec<Value>),
    Tuple(Vec<Value>),
}

impl Value {
    pub(crate) fn kind_name(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::Bool(_) => "bool",
            Self::Object(_) => "object",
            Self::List(_) => "list",
            Self::Tuple(_) => "tuple",
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::String(value) => serializer.serialize_str(value),
            Self::Int(value) => serializer.serialize_i64(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Object(values) => {
                let mut map = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
            Self::List(values) | Self::Tuple(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(value)?;
                }
                sequence.end()
            }
        }
    }
}

pub fn compile_schema(source: Source) -> Result<Schema, ErrorReport> {
    let ast = syntax::parse_schema(&source)?;
    schema::compile(source, ast)
}

pub fn parse_overlay(source: Source) -> Result<OverlayDocument, ErrorReport> {
    let ast = syntax::parse_overlay(&source)?;
    Ok(OverlayDocument { source, ast })
}

pub fn evaluate(schema: &Schema, overlays: &[OverlayDocument]) -> Result<Value, ErrorReport> {
    eval::evaluate(schema, overlays)
}

pub fn parse_json_value(schema: &Schema, source: Source) -> Result<Value, ErrorReport> {
    json::parse(schema, source)
}

pub fn evaluate_with_initial(
    schema: &Schema,
    initial: &Value,
    overlays: &[OverlayDocument],
) -> Result<Value, ErrorReport> {
    eval::evaluate_with_initial(schema, initial, overlays)
}

pub fn evaluate_sources(schema: Source, overlays: Vec<Source>) -> Result<Value, ErrorReport> {
    let schema = compile_schema(schema)?;
    let overlays = overlays
        .into_iter()
        .map(parse_overlay)
        .collect::<Result<Vec<_>, _>>()?;
    evaluate(&schema, &overlays)
}

pub fn evaluate_sources_with_initial(
    schema: Source,
    initial: &Value,
    overlays: Vec<Source>,
) -> Result<Value, ErrorReport> {
    let schema = compile_schema(schema)?;
    let overlays = overlays
        .into_iter()
        .map(parse_overlay)
        .collect::<Result<Vec<_>, _>>()?;
    evaluate_with_initial(&schema, initial, &overlays)
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = serde_json::to_string_pretty(self).map_err(|_| fmt::Error)?;
        f.write_str(&json)
    }
}

#[repr(C)]
pub struct OonSource {
    pub name: *const u8,
    pub name_len: usize,
    pub text: *const u8,
    pub text_len: usize,
}

#[repr(C)]
pub struct OonOutput {
    pub status: u32,
    pub bytes: *mut u8,
    pub len: usize,
}

#[repr(C)]
pub struct OonValue {
    raw: serde_json::Value,
}

#[unsafe(no_mangle)]
pub extern "C" fn oon_abi_version() -> u32 {
    1
}

#[unsafe(no_mangle)]
/// Evaluates copied OON sources through the version-1 C ABI.
///
/// # Safety
///
/// `schema` must point to one valid `OonSource`. If `count` is nonzero,
/// `overlays` must point to an array of `count` valid values. Every nonempty
/// byte range referenced by those values must remain readable for this call.
pub unsafe extern "C" fn oon_evaluate_v1(
    schema: *const OonSource,
    overlays: *const OonSource,
    count: usize,
) -> OonOutput {
    match std::panic::catch_unwind(|| ffi_evaluate(schema, overlays, count)) {
        Ok(Ok(value)) => output(0, value),
        Ok(Err((status, value))) => output(status, value),
        Err(_) => output(3, "internal panic".to_owned()),
    }
}

#[unsafe(no_mangle)]
/// Parses a copied JSON source into an immutable OON value handle.
///
/// On success, `out_value` receives an owned handle and the returned output has
/// status zero and no bytes. Release the handle with [`oon_value_free`].
///
/// # Safety
///
/// `json` and `out_value` must be valid writable/readable pointers for this
/// call. Every nonempty byte range in `json` must remain readable for the call.
pub unsafe extern "C" fn oon_value_from_json_v1(
    json: *const OonSource,
    out_value: *mut *mut OonValue,
) -> OonOutput {
    match std::panic::catch_unwind(|| ffi_value_from_json(json, out_value)) {
        Ok(Ok(())) => empty_output(),
        Ok(Err((status, value))) => output(status, value),
        Err(_) => output(3, "internal panic".to_owned()),
    }
}

fn ffi_value_from_json(
    json: *const OonSource,
    out_value: *mut *mut OonValue,
) -> Result<(), (u32, String)> {
    if out_value.is_null() {
        return Err((2, "invalid null argument".into()));
    }
    // SAFETY: validated non-null; the caller promises a writable output pointer.
    unsafe {
        *out_value = std::ptr::null_mut();
    }
    if json.is_null() {
        return Err((2, "invalid null argument".into()));
    }
    // SAFETY: pointer and lengths are covered by the documented C contract.
    let source = unsafe { copy_source(&*json) }.map_err(|value| (2, value))?;
    let raw = json::parse_raw(&source).map_err(|report| (1, report.to_string()))?;
    let value = Box::into_raw(Box::new(OonValue { raw }));
    // SAFETY: validated non-null and this transfers the new allocation to the caller.
    unsafe {
        *out_value = value;
    }
    Ok(())
}

#[unsafe(no_mangle)]
/// Evaluates copied OON sources from an immutable parsed value handle.
///
/// The handle is borrowed and may be reused, including by concurrent calls.
///
/// # Safety
///
/// `schema` and `value` must point to valid values. If `count` is nonzero,
/// `overlays` must point to an array of `count` valid sources. Referenced byte
/// ranges and the value handle must remain alive for this call.
pub unsafe extern "C" fn oon_evaluate_value_v1(
    schema: *const OonSource,
    value: *const OonValue,
    overlays: *const OonSource,
    count: usize,
) -> OonOutput {
    match std::panic::catch_unwind(|| ffi_evaluate_value(schema, value, overlays, count)) {
        Ok(Ok(value)) => output(0, value),
        Ok(Err((status, value))) => output(status, value),
        Err(_) => output(3, "internal panic".to_owned()),
    }
}

fn ffi_evaluate_value(
    schema: *const OonSource,
    value: *const OonValue,
    overlays: *const OonSource,
    count: usize,
) -> Result<String, (u32, String)> {
    if schema.is_null() || value.is_null() || (count != 0 && overlays.is_null()) {
        return Err((2, "invalid null argument".into()));
    }
    // SAFETY: pointers and lengths are part of the documented C contract.
    let schema = unsafe { copy_source(&*schema) }.map_err(|value| (2, value))?;
    let overlay_slice = if count == 0 {
        &[][..]
    } else {
        // SAFETY: the caller promises an array of `count` elements.
        unsafe { std::slice::from_raw_parts(overlays, count) }
    };
    let mut copied = Vec::with_capacity(count);
    for source in overlay_slice {
        // SAFETY: each source is covered by the caller's array contract.
        copied.push(unsafe { copy_source(source) }.map_err(|value| (2, value))?);
    }

    let schema = compile_schema(schema).map_err(|report| (1, report.to_string()))?;
    let overlays = copied
        .into_iter()
        .map(parse_overlay)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|report| (1, report.to_string()))?;
    // SAFETY: the caller promises that the immutable handle remains alive.
    let initial = json::decode(&schema, unsafe { &(*value).raw })
        .map_err(|report| (1, report.to_string()))?;
    let value = evaluate_with_initial(&schema, &initial, &overlays)
        .map_err(|report| (1, report.to_string()))?;
    serde_json::to_string_pretty(&value)
        .map(|value| value + "\n")
        .map_err(|error| (3, error.to_string()))
}

fn ffi_evaluate(
    schema: *const OonSource,
    overlays: *const OonSource,
    count: usize,
) -> Result<String, (u32, String)> {
    if schema.is_null() || (count != 0 && overlays.is_null()) {
        return Err((2, "invalid null argument".into()));
    }
    // SAFETY: pointers and lengths are part of the documented C contract and are copied immediately.
    let schema = unsafe { copy_source(&*schema) }.map_err(|value| (2, value))?;
    // SAFETY: the caller promises an array of `count` elements when count is nonzero.
    let overlay_slice = if count == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(overlays, count) }
    };
    let mut copied = Vec::with_capacity(count);
    for source in overlay_slice {
        copied.push(unsafe { copy_source(source) }.map_err(|value| (2, value))?);
    }
    let value = evaluate_sources(schema, copied).map_err(|report| (1, report.to_string()))?;
    serde_json::to_string_pretty(&value)
        .map(|value| value + "\n")
        .map_err(|error| (3, error.to_string()))
}

unsafe fn copy_source(source: &OonSource) -> Result<Source, String> {
    if (source.name_len != 0 && source.name.is_null())
        || (source.text_len != 0 && source.text.is_null())
    {
        return Err("invalid null source pointer".into());
    }
    // SAFETY: validated non-null for nonempty lengths; caller owns readable buffers for this call.
    let name = unsafe { std::slice::from_raw_parts(source.name, source.name_len) };
    // SAFETY: same contract as above.
    let text = unsafe { std::slice::from_raw_parts(source.text, source.text_len) };
    Ok(Source {
        name: std::str::from_utf8(name)
            .map_err(|_| "source name is not UTF-8")?
            .to_owned(),
        text: std::str::from_utf8(text)
            .map_err(|_| "source text is not UTF-8")?
            .to_owned(),
    })
}

fn output(status: u32, value: String) -> OonOutput {
    let bytes = value.into_bytes().into_boxed_slice();
    let len = bytes.len();
    let bytes = Box::into_raw(bytes).cast::<u8>();
    OonOutput { status, bytes, len }
}

fn empty_output() -> OonOutput {
    OonOutput {
        status: 0,
        bytes: std::ptr::null_mut(),
        len: 0,
    }
}

#[unsafe(no_mangle)]
/// Releases an OON value returned through [`oon_value_from_json_v1`].
///
/// A null pointer is accepted and does nothing.
///
/// # Safety
///
/// `value` must be null or an as-yet-unfreed handle allocated by this library,
/// and no evaluation may still be borrowing it.
pub unsafe extern "C" fn oon_value_free(value: *mut OonValue) {
    if !value.is_null() {
        // SAFETY: this reconstructs the allocation transferred to the caller.
        unsafe {
            drop(Box::from_raw(value));
        }
    }
}

#[unsafe(no_mangle)]
/// Releases an output returned by [`oon_evaluate_v1`].
///
/// # Safety
///
/// `output` must be an as-yet-unfreed value returned by this library.
pub unsafe extern "C" fn oon_output_free(output: OonOutput) {
    if !output.bytes.is_null() {
        // SAFETY: this reconstructs the exact boxed slice allocated by `output`.
        let slice = std::ptr::slice_from_raw_parts_mut(output.bytes, output.len);
        unsafe {
            drop(Box::from_raw(slice));
        }
    }
}
