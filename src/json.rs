// SPDX-License-Identifier: MPL-2.0

use indexmap::IndexMap;

use crate::diagnostic::{Diagnostic, ErrorReport, Span};
use crate::schema::{Schema, TypeId, TypeNode};
use crate::{Source, Value};

pub(crate) fn parse_raw(source: &Source) -> Result<serde_json::Value, ErrorReport> {
    serde_json::from_str(&source.text).map_err(|error| ErrorReport {
        diagnostics: vec![Diagnostic {
            source: source.name.clone(),
            line: error.line(),
            column: error.column(),
            phase: "JSON".into(),
            message: error.to_string(),
            overlay: None,
            declaration: None,
            path: None,
        }],
    })
}

pub(crate) fn parse(schema: &Schema, source: Source) -> Result<Value, ErrorReport> {
    let raw = parse_raw(&source)?;
    decode(schema, &raw)
}

pub(crate) fn decode(schema: &Schema, raw: &serde_json::Value) -> Result<Value, ErrorReport> {
    decode_at(schema, schema.root, raw).map_err(|message| {
        schema.report(
            Span::default(),
            format!("initial validation failed: {message}"),
        )
    })
}

fn decode_at(schema: &Schema, ty: TypeId, raw: &serde_json::Value) -> Result<Value, String> {
    let resolved = schema.resolved(ty)?;
    match &schema.types[resolved] {
        TypeNode::String => raw
            .as_str()
            .map(|value| Value::String(value.to_owned()))
            .ok_or_else(|| expected("string", raw)),
        TypeNode::Int => raw
            .as_i64()
            .map(Value::Int)
            .ok_or_else(|| expected("signed 64-bit integer", raw)),
        TypeNode::Float => {
            if !raw.as_number().is_some_and(serde_json::Number::is_f64) {
                return Err(expected("float", raw));
            }
            let value = raw.as_f64().ok_or_else(|| expected("finite float", raw))?;
            if !value.is_finite() {
                return Err("expected finite float".into());
            }
            Ok(Value::Float(value))
        }
        TypeNode::Bool => raw
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| expected("bool", raw)),
        TypeNode::Any => decode_any(raw),
        TypeNode::Literal(expected_value) => {
            let value = decode_like(expected_value, raw)?;
            if &value == expected_value {
                Ok(value)
            } else {
                Err("value does not match literal type".into())
            }
        }
        TypeNode::Object(fields) => {
            let object = raw.as_object().ok_or_else(|| expected("object", raw))?;
            let mut values = IndexMap::new();
            for (key, raw_value) in object {
                let field = fields
                    .iter()
                    .find(|field| field.name == *key)
                    .ok_or_else(|| format!("unknown fixed-object field `{key}`"))?;
                values.insert(key.clone(), decode_at(schema, field.ty, raw_value)?);
            }
            Ok(Value::Object(values))
        }
        TypeNode::Map(inner) => {
            let object = raw.as_object().ok_or_else(|| expected("map", raw))?;
            let mut values = IndexMap::new();
            for (key, raw_value) in object {
                values.insert(key.clone(), decode_at(schema, *inner, raw_value)?);
            }
            Ok(Value::Object(values))
        }
        TypeNode::List { item, key } => {
            let array = raw.as_array().ok_or_else(|| expected("list", raw))?;
            let mut values = Vec::with_capacity(array.len());
            for raw_value in array {
                let value = decode_at(schema, *item, raw_value)?;
                if let Some(key) = key {
                    let Value::Object(object) = &value else {
                        return Err("keyed-list item must be an object".into());
                    };
                    if !object.contains_key(key) {
                        return Err(format!("keyed-list item must explicitly contain `{key}`"));
                    }
                }
                values.push(value);
            }
            Ok(Value::List(values))
        }
        TypeNode::Tuple(items) => {
            let array = raw.as_array().ok_or_else(|| expected("tuple", raw))?;
            if array.len() != items.len() {
                return Err(format!("tuple requires {} elements", items.len()));
            }
            let values = items
                .iter()
                .zip(array)
                .map(|(item, raw_value)| decode_at(schema, *item, raw_value))
                .collect::<Result<_, _>>()?;
            Ok(Value::Tuple(values))
        }
        TypeNode::Union(branches) => {
            let matches = branches
                .iter()
                .filter_map(|branch| decode_at(schema, *branch, raw).ok())
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [value] => Ok(value.clone()),
                [] => Err("value matches no union branch".into()),
                _ => Err("value ambiguously matches multiple union branches".into()),
            }
        }
        TypeNode::Tagged { tag, variants } => {
            let object = raw
                .as_object()
                .ok_or_else(|| expected("tagged object", raw))?;
            let discriminator = object
                .get(tag)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    format!("tagged value must explicitly contain string discriminator `{tag}`")
                })?;
            let normalized = discriminator.to_ascii_lowercase();
            let variant = variants
                .iter()
                .find(|variant| variant.name == normalized)
                .ok_or_else(|| format!("unknown tagged variant `{discriminator}`"))?;
            let mut values = IndexMap::new();
            values.insert(tag.clone(), Value::String(normalized));
            for (key, raw_value) in object {
                if key == tag {
                    continue;
                }
                let field = variant
                    .fields
                    .iter()
                    .find(|field| field.name == *key)
                    .ok_or_else(|| format!("unknown tagged field `{key}`"))?;
                values.insert(key.clone(), decode_at(schema, field.ty, raw_value)?);
            }
            Ok(Value::Object(values))
        }
        TypeNode::Pending | TypeNode::Ref(_) => unreachable!("resolved"),
    }
}

fn decode_any(raw: &serde_json::Value) -> Result<Value, String> {
    match raw {
        serde_json::Value::Null => Err("JSON null is not an OON value".into()),
        serde_json::Value::Bool(value) => Ok(Value::Bool(*value)),
        serde_json::Value::Number(value) if value.is_i64() => {
            Ok(Value::Int(value.as_i64().expect("integer")))
        }
        serde_json::Value::Number(value) if value.is_f64() => {
            let value = value.as_f64().expect("float");
            if value.is_finite() {
                Ok(Value::Float(value))
            } else {
                Err("expected finite float".into())
            }
        }
        serde_json::Value::Number(_) => Err("integer is outside the signed 64-bit range".into()),
        serde_json::Value::String(value) => Ok(Value::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .map(decode_any)
            .collect::<Result<_, _>>()
            .map(Value::List),
        serde_json::Value::Object(entries) => {
            let values = entries
                .iter()
                .map(|(key, value)| Ok((key.clone(), decode_any(value)?)))
                .collect::<Result<IndexMap<_, _>, String>>()?;
            Ok(Value::Object(values))
        }
    }
}

fn decode_like(expected: &Value, raw: &serde_json::Value) -> Result<Value, String> {
    match expected {
        Value::String(_) => raw
            .as_str()
            .map(|value| Value::String(value.to_owned()))
            .ok_or_else(|| expected_kind("string", raw)),
        Value::Int(_) => raw
            .as_i64()
            .map(Value::Int)
            .ok_or_else(|| expected_kind("signed 64-bit integer", raw)),
        Value::Float(_) => {
            if raw.as_number().is_some_and(serde_json::Number::is_f64) {
                Ok(Value::Float(raw.as_f64().expect("float")))
            } else {
                Err(expected_kind("float", raw))
            }
        }
        Value::Bool(_) => raw
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| expected_kind("bool", raw)),
        Value::Object(_) | Value::List(_) | Value::Tuple(_) => {
            Err("composite literal types are not supported".into())
        }
    }
}

fn expected(kind: &str, raw: &serde_json::Value) -> String {
    expected_kind(kind, raw)
}

fn expected_kind(kind: &str, raw: &serde_json::Value) -> String {
    format!("expected {kind}, got {}", json_kind(raw))
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(value) if value.is_i64() => "int",
        serde_json::Value::Number(value) if value.is_u64() => "unsigned integer",
        serde_json::Value::Number(_) => "float",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
