// SPDX-License-Identifier: MPL-2.0

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::diagnostic::{ErrorReport, Span, diagnostic};
use crate::syntax::{Declaration, FieldExpr, ObjectShape, SchemaAst, TyExpr};
use crate::{Source, Value};

pub(crate) type TypeId = usize;

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub name: String,
    pub optional: bool,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) struct Variant {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug)]
pub(crate) enum TypeNode {
    Pending,
    Ref(TypeId),
    String,
    Int,
    Float,
    Bool,
    Any,
    Literal(Value),
    Object(Vec<Field>),
    Map(TypeId),
    List { item: TypeId, key: Option<String> },
    Tuple(Vec<TypeId>),
    Union(Vec<TypeId>),
    Tagged { tag: String, variants: Vec<Variant> },
}

#[derive(Clone, Debug)]
pub struct Schema {
    pub(crate) source: Source,
    pub(crate) name: String,
    pub(crate) root: TypeId,
    pub(crate) types: Vec<TypeNode>,
}

struct Builder<'a> {
    source: &'a Source,
    types: Vec<TypeNode>,
    symbols: HashMap<String, TypeId>,
    definitions: HashMap<TypeId, TyExpr>,
    building: HashSet<TypeId>,
}

impl<'a> Builder<'a> {
    fn alloc(&mut self, node: TypeNode) -> TypeId {
        let id = self.types.len();
        self.types.push(node);
        id
    }

    fn ensure_named(&mut self, id: TypeId) -> Result<(), ErrorReport> {
        if !matches!(self.types[id], TypeNode::Pending) {
            return Ok(());
        }
        if !self.building.insert(id) {
            return self.fail(Span::default(), "recursive object-shape dependency");
        }
        let expression = self.definitions[&id].clone();
        let target = self.build_ty(&expression)?;
        self.types[id] = TypeNode::Ref(target);
        self.building.remove(&id);
        Ok(())
    }

    fn ensure_resolved(&mut self, mut id: TypeId) -> Result<TypeId, ErrorReport> {
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(id) {
                return self.fail(Span::default(), "recursive object-shape alias");
            }
            self.ensure_named(id)?;
            match self.types[id] {
                TypeNode::Ref(next) => id = next,
                _ => return Ok(id),
            }
        }
    }

    fn build_ty(&mut self, expression: &TyExpr) -> Result<TypeId, ErrorReport> {
        let node = match expression {
            TyExpr::String(_) => TypeNode::String,
            TyExpr::Int(_) => TypeNode::Int,
            TyExpr::Float(_) => TypeNode::Float,
            TyExpr::Bool(_) => TypeNode::Bool,
            TyExpr::Any(_) => TypeNode::Any,
            TyExpr::Literal(value, _) => TypeNode::Literal(value.clone()),
            TyExpr::Name(name, span) => {
                let Some(id) = self.symbols.get(name) else {
                    return self.fail(*span, format!("unknown type `{name}`"));
                };
                TypeNode::Ref(*id)
            }
            TyExpr::Object(fields, _) => TypeNode::Object(self.build_fields(fields)?),
            TyExpr::Map(inner, _) => {
                let inner = self.build_ty(inner)?;
                TypeNode::Map(inner)
            }
            TyExpr::List(inner, _) => {
                let item = self.build_ty(inner)?;
                TypeNode::List { item, key: None }
            }
            TyExpr::Tuple(items, _) => {
                let mut built = Vec::with_capacity(items.len());
                for item in items {
                    built.push(self.build_ty(item)?);
                }
                TypeNode::Tuple(built)
            }
            TyExpr::Union(branches, _) => {
                let mut built = Vec::with_capacity(branches.len());
                for branch in branches {
                    built.push(self.build_ty(branch)?);
                }
                TypeNode::Union(built)
            }
            TyExpr::Keyed(inner, key, span) => {
                let id = self.build_ty(inner)?;
                let resolved = self.ensure_resolved(id)?;
                let TypeNode::List { item, .. } = self.types[resolved].clone() else {
                    return self.fail(*span, "`key` may only follow a list type");
                };
                TypeNode::List {
                    item,
                    key: Some(key.clone()),
                }
            }
            TyExpr::Tagged {
                tag,
                common,
                variants,
                span,
            } => {
                let common_fields = if let Some(shape) = common {
                    self.shape_fields(shape)?
                } else {
                    Vec::new()
                };
                ensure_unique_fields(self.source, &common_fields)?;
                if common_fields.iter().any(|field| field.name == *tag) {
                    return self.fail(*span, "tagged common payload redeclares discriminator");
                }
                let mut seen = HashSet::new();
                let mut built = Vec::new();
                for (name, _, shape, variant_span) in variants {
                    if !seen.insert(name.clone()) {
                        return self
                            .fail(*variant_span, format!("duplicate tagged variant `{name}`"));
                    }
                    let payload = self.shape_fields(shape)?;
                    ensure_unique_fields(self.source, &payload)?;
                    for field in &payload {
                        if field.name == *tag
                            || common_fields.iter().any(|common| common.name == field.name)
                        {
                            return self.fail(
                                field.span,
                                format!("variant `{name}` redeclares `{}`", field.name),
                            );
                        }
                    }
                    let mut fields = common_fields.clone();
                    fields.extend(payload);
                    built.push(Variant {
                        name: name.clone(),
                        fields,
                    });
                }
                TypeNode::Tagged {
                    tag: tag.clone(),
                    variants: built,
                }
            }
        };
        Ok(self.alloc(node))
    }

    fn shape_fields(&mut self, shape: &ObjectShape) -> Result<Vec<Field>, ErrorReport> {
        match shape {
            ObjectShape::Inline(fields) => self.build_fields(fields),
            ObjectShape::Name(name, span) => {
                let Some(id) = self.symbols.get(name).copied() else {
                    return self.fail(*span, format!("unknown object type `{name}`"));
                };
                let resolved = self.ensure_resolved(id)?;
                match &self.types[resolved] {
                    TypeNode::Object(fields) => Ok(fields.clone()),
                    TypeNode::Pending => unreachable!("resolved named shape"),
                    _ => self.fail(
                        *span,
                        format!("`{name}` does not resolve to a fixed object"),
                    ),
                }
            }
        }
    }

    fn build_fields(&mut self, fields: &[FieldExpr]) -> Result<Vec<Field>, ErrorReport> {
        let mut built = Vec::new();
        let mut seen = HashSet::new();
        for field in fields {
            if !seen.insert(field.name.clone()) {
                return self.fail(
                    field.span,
                    format!("duplicate field `{}` after normalization", field.spelling),
                );
            }
            let ty = self.build_ty(&field.ty)?;
            built.push(Field {
                name: field.name.clone(),
                optional: field.optional,
                ty,
                span: field.span,
            });
        }
        Ok(built)
    }

    fn fail<T>(&self, span: Span, message: impl Into<String>) -> Result<T, ErrorReport> {
        Err(ErrorReport::one(diagnostic(
            self.source,
            span,
            "schema",
            message,
        )))
    }
}

fn ensure_unique_fields(source: &Source, fields: &[Field]) -> Result<(), ErrorReport> {
    let mut seen = HashSet::new();
    for field in fields {
        if !seen.insert(&field.name) {
            return Err(ErrorReport::one(diagnostic(
                source,
                field.span,
                "schema",
                format!("duplicate field `{}`", field.name),
            )));
        }
    }
    Ok(())
}

pub(crate) fn compile(source: Source, ast: SchemaAst) -> Result<Schema, ErrorReport> {
    let mut symbols = HashMap::new();
    let mut declarations = Vec::new();
    let mut schema_declaration = None;
    let mut types = vec![
        TypeNode::String,
        TypeNode::Int,
        TypeNode::Float,
        TypeNode::Bool,
        TypeNode::Any,
    ];
    for declaration in &ast.declarations {
        let (name, expression, is_schema) = match &declaration.value {
            Declaration::Type(name, expression) => (name, expression, false),
            Declaration::Schema(name, expression) => (name, expression, true),
        };
        if matches!(name.as_str(), "string" | "int" | "float" | "bool" | "any") {
            return Err(ErrorReport::one(diagnostic(
                &source,
                declaration.span,
                "schema",
                format!("reserved name `{name}` cannot be declared"),
            )));
        }
        if symbols.contains_key(name) {
            return Err(ErrorReport::one(diagnostic(
                &source,
                declaration.span,
                "schema",
                format!("duplicate declaration `{name}` after normalization"),
            )));
        }
        if is_schema {
            if schema_declaration.is_some() {
                return Err(ErrorReport::one(diagnostic(
                    &source,
                    declaration.span,
                    "schema",
                    "a schema document must contain exactly one schema declaration",
                )));
            }
            schema_declaration = Some((name.clone(), expression, declaration.span));
        }
        let id = types.len();
        types.push(TypeNode::Pending);
        symbols.insert(name.clone(), id);
        declarations.push((id, expression));
    }
    let Some((schema_name, _, schema_span)) = schema_declaration else {
        return Err(ErrorReport::one(diagnostic(
            &source,
            Span::default(),
            "schema",
            "a schema document must contain exactly one schema declaration",
        )));
    };
    let definitions = declarations
        .iter()
        .map(|(id, expression)| (*id, (*expression).clone()))
        .collect();
    let mut builder = Builder {
        source: &source,
        types,
        symbols,
        definitions,
        building: HashSet::new(),
    };
    for (id, _) in declarations {
        builder.ensure_named(id)?;
    }
    let root = *builder.symbols.get(&schema_name).expect("schema symbol");
    let built_types = std::mem::take(&mut builder.types);
    drop(builder);
    let schema = Schema {
        source,
        name: schema_name,
        root,
        types: built_types,
    };
    schema.validate_schema(schema_span)?;
    Ok(schema)
}

impl Schema {
    fn validate_schema(&self, span: Span) -> Result<(), ErrorReport> {
        for id in 0..self.types.len() {
            if let TypeNode::List {
                item,
                key: Some(key),
            } = &self.types[id]
            {
                match self.node(*item)? {
                    TypeNode::Object(fields) => {
                        let Some(field) = fields.iter().find(|field| &field.name == key) else {
                            return Err(self.report(
                                span,
                                format!("keyed-list identity `{key}` is not an item field"),
                            ));
                        };
                        self.validate_identity_field(field)?;
                    }
                    TypeNode::Tagged { tag, variants } => {
                        if key == tag {
                            continue;
                        }
                        for variant in variants {
                            let Some(field) =
                                variant.fields.iter().find(|field| &field.name == key)
                            else {
                                return Err(self.report(
                                    span,
                                    format!(
                                        "keyed-list identity `{key}` is not present in every tagged variant"
                                    ),
                                ));
                            };
                            self.validate_identity_field(field)?;
                        }
                    }
                    _ => return Err(self.report(span, "type is not object-shaped")),
                }
            }
        }
        self.canonical(self.root)
            .map(|_| ())
            .map_err(|message| self.report(span, message))
    }

    pub(crate) fn report(&self, span: Span, message: impl Into<String>) -> ErrorReport {
        ErrorReport::one(diagnostic(&self.source, span, "schema", message))
    }

    pub(crate) fn resolved(&self, id: TypeId) -> Result<TypeId, String> {
        resolve_node_in(&self.types, id)
    }
    pub(crate) fn node(&self, id: TypeId) -> Result<&TypeNode, ErrorReport> {
        let id = self
            .resolved(id)
            .map_err(|message| self.report(Span::default(), message))?;
        Ok(&self.types[id])
    }
    fn validate_identity_field(&self, field: &Field) -> Result<(), ErrorReport> {
        if field.optional || !matches!(self.node(field.ty)?, TypeNode::String | TypeNode::Int) {
            return Err(self.report(
                field.span,
                "keyed-list identity must be a required string or int field",
            ));
        }
        Ok(())
    }

    pub(crate) fn canonical(&self, id: TypeId) -> Result<Value, String> {
        self.canonical_inner(id, &mut Vec::new())
    }
    fn canonical_inner(&self, id: TypeId, visiting: &mut Vec<TypeId>) -> Result<Value, String> {
        let id = self.resolved(id)?;
        if visiting.contains(&id) {
            return Err("canonical construction contains a recursive dependency".into());
        }
        visiting.push(id);
        let value = match &self.types[id] {
            TypeNode::String => Value::String(String::new()),
            TypeNode::Int => Value::Int(0),
            TypeNode::Float => Value::Float(0.0),
            TypeNode::Bool => Value::Bool(false),
            TypeNode::Any => return Err("canonical construction reaches `any`".into()),
            TypeNode::Literal(value) => value.clone(),
            TypeNode::Object(fields) => {
                let mut value = IndexMap::new();
                for field in fields.iter().filter(|field| !field.optional) {
                    value.insert(
                        field.name.clone(),
                        self.canonical_inner(field.ty, visiting)?,
                    );
                }
                Value::Object(value)
            }
            TypeNode::Map(_) | TypeNode::List { .. } => match &self.types[id] {
                TypeNode::Map(_) => Value::Object(IndexMap::new()),
                _ => Value::List(Vec::new()),
            },
            TypeNode::Tuple(items) => {
                let mut values = Vec::new();
                for item in items {
                    values.push(self.canonical_inner(*item, visiting)?);
                }
                Value::Tuple(values)
            }
            TypeNode::Union(branches) => self.canonical_inner(branches[0], visiting)?,
            TypeNode::Tagged { tag, variants } => {
                let variant = &variants[0];
                let mut value = IndexMap::new();
                value.insert(tag.clone(), Value::String(variant.name.clone()));
                for field in variant.fields.iter().filter(|field| !field.optional) {
                    value.insert(
                        field.name.clone(),
                        self.canonical_inner(field.ty, visiting)?,
                    );
                }
                Value::Object(value)
            }
            TypeNode::Pending | TypeNode::Ref(_) => unreachable!("resolved"),
        };
        visiting.pop();
        Ok(value)
    }

    pub(crate) fn validate_complete(&self, id: TypeId, value: &Value) -> Result<(), String> {
        self.validate_value(id, value, false)
    }

    pub(crate) fn validate_value(
        &self,
        id: TypeId,
        value: &Value,
        supplied: bool,
    ) -> Result<(), String> {
        let id = self.resolved(id)?;
        match &self.types[id] {
            TypeNode::String if matches!(value, Value::String(_)) => Ok(()),
            TypeNode::Int if matches!(value, Value::Int(_)) => Ok(()),
            TypeNode::Float if matches!(value, Value::Float(v) if v.is_finite()) => Ok(()),
            TypeNode::Bool if matches!(value, Value::Bool(_)) => Ok(()),
            TypeNode::Any => Ok(()),
            TypeNode::Literal(expected) if expected == value => Ok(()),
            TypeNode::Object(fields) => {
                let Value::Object(values) = value else {
                    return Err("expected object".into());
                };
                for key in values.keys() {
                    if !fields.iter().any(|field| &field.name == key) {
                        return Err(format!("unknown fixed-object field `{key}`"));
                    }
                }
                for field in fields {
                    match values.get(&field.name) {
                        Some(value) => self.validate_value(field.ty, value, supplied)?,
                        None if !field.optional && !supplied => {
                            return Err(format!("missing required field `{}`", field.name));
                        }
                        None => {}
                    }
                }
                Ok(())
            }
            TypeNode::Map(inner) => {
                let Value::Object(values) = value else {
                    return Err("expected map".into());
                };
                for value in values.values() {
                    self.validate_value(*inner, value, supplied)?;
                }
                Ok(())
            }
            TypeNode::List { item, key } => {
                let Value::List(values) = value else {
                    return Err("expected list".into());
                };
                let mut identities = HashSet::new();
                for value in values {
                    self.validate_value(*item, value, supplied)?;
                    if let Some(key) = key {
                        let Value::Object(object) = value else {
                            return Err("keyed-list item must be an object".into());
                        };
                        let Some(identity) = object.get(key) else {
                            return Err(format!("keyed-list item must explicitly contain `{key}`"));
                        };
                        let encoded = identity_key(identity)?;
                        if !identities.insert(encoded) {
                            return Err("duplicate keyed-list identity".into());
                        }
                    }
                }
                Ok(())
            }
            TypeNode::Tuple(items) => {
                let Value::Tuple(values) = value else {
                    return Err("expected tuple".into());
                };
                if items.len() != values.len() {
                    return Err(format!("tuple requires {} elements", items.len()));
                }
                for (ty, value) in items.iter().zip(values) {
                    self.validate_value(*ty, value, supplied)?;
                }
                Ok(())
            }
            TypeNode::Union(branches) => {
                let matches = branches
                    .iter()
                    .filter(|branch| self.validate_value(**branch, value, supplied).is_ok())
                    .count();
                match matches {
                    1 => Ok(()),
                    _ if !supplied && matches > 1 => Ok(()),
                    0 => Err("value matches no union branch".into()),
                    _ => Err("value ambiguously matches multiple union branches".into()),
                }
            }
            TypeNode::Tagged { tag, variants } => {
                let Value::Object(values) = value else {
                    return Err("expected tagged object".into());
                };
                let Some(Value::String(discriminator)) = values.get(tag) else {
                    return Err(format!(
                        "tagged value must explicitly contain string discriminator `{tag}`"
                    ));
                };
                let discriminator = discriminator.to_ascii_lowercase();
                let Some(variant) = variants
                    .iter()
                    .find(|variant| variant.name == discriminator)
                else {
                    return Err(format!("unknown tagged variant `{discriminator}`"));
                };
                for key in values.keys() {
                    if key != tag && !variant.fields.iter().any(|field| &field.name == key) {
                        return Err(format!("unknown tagged field `{key}`"));
                    }
                }
                for field in &variant.fields {
                    match values.get(&field.name) {
                        Some(value) => self.validate_value(field.ty, value, supplied)?,
                        None if !field.optional && !supplied => {
                            return Err(format!("missing required field `{}`", field.name));
                        }
                        None => {}
                    }
                }
                Ok(())
            }
            _ => Err(format!(
                "expected {}, got {}",
                self.type_name(id),
                value.kind_name()
            )),
        }
    }

    pub(crate) fn type_name(&self, id: TypeId) -> &'static str {
        let Ok(id) = self.resolved(id) else {
            return "recursive alias";
        };
        match self.types[id] {
            TypeNode::String => "string",
            TypeNode::Int => "int",
            TypeNode::Float => "float",
            TypeNode::Bool => "bool",
            TypeNode::Any => "any",
            TypeNode::Literal(_) => "literal",
            TypeNode::Object(_) => "object",
            TypeNode::Map(_) => "map",
            TypeNode::List { .. } => "list",
            TypeNode::Tuple(_) => "tuple",
            TypeNode::Union(_) => "union",
            TypeNode::Tagged { .. } => "tagged union",
            _ => "type",
        }
    }
}

fn resolve_node_in(types: &[TypeNode], mut id: TypeId) -> Result<TypeId, String> {
    let mut seen = HashSet::new();
    while let TypeNode::Ref(next) = types
        .get(id)
        .ok_or_else(|| "invalid type reference".to_owned())?
    {
        if !seen.insert(id) {
            return Err("recursive alias cycle".into());
        }
        id = *next;
    }
    Ok(id)
}

pub(crate) fn identity_key(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(format!("s:{value}")),
        Value::Int(value) => Ok(format!("i:{value}")),
        _ => Err("key identity must be string or int".into()),
    }
}
