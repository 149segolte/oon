// SPDX-License-Identifier: MPL-2.0

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use crate::diagnostic::{ErrorReport, Span, diagnostic};
use crate::schema::{Field, Schema, TypeId, TypeNode, identity_key};
use crate::syntax::{
    ActionKind, BinaryOp, Expr, ExprKind, Node, Overlay, Path, PathSegment, Statement, UnaryOp,
};
use crate::{OverlayDocument, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VarType {
    Int,
    String,
}

type Scopes = Vec<HashMap<String, VarType>>;
type Vars = Vec<HashMap<String, Value>>;

struct Context<'a> {
    schema: &'a Schema,
    source: &'a crate::Source,
    overlay: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathUse {
    Read,
    Target,
}

impl Context<'_> {
    fn fail<T>(
        &self,
        span: Span,
        phase: &str,
        message: impl Into<String>,
    ) -> Result<T, ErrorReport> {
        let mut value = diagnostic(self.source, span, phase, message);
        value.overlay = self.overlay.map(str::to_owned);
        Err(ErrorReport::one(value))
    }
}

pub(crate) fn evaluate(
    schema: &Schema,
    overlays: &[OverlayDocument],
) -> Result<Value, ErrorReport> {
    evaluate_inner(schema, None, overlays)
}

pub(crate) fn evaluate_with_initial(
    schema: &Schema,
    initial: &Value,
    overlays: &[OverlayDocument],
) -> Result<Value, ErrorReport> {
    evaluate_inner(schema, Some(initial), overlays)
}

fn evaluate_inner(
    schema: &Schema,
    initial: Option<&Value>,
    overlays: &[OverlayDocument],
) -> Result<Value, ErrorReport> {
    for document in overlays {
        let ctx = Context {
            schema,
            source: &document.source,
            overlay: None,
        };
        if document.ast.locator.value.to_ascii_lowercase() != schema.name {
            return ctx.fail(
                document.ast.locator.span,
                "validation",
                format!(
                    "schema locator `{}` does not match `{}`",
                    document.ast.locator.value, schema.name
                ),
            );
        }
        for overlay in &document.ast.overlays {
            let ctx = Context {
                schema,
                source: &document.source,
                overlay: Some(&overlay.value.spelling),
            };
            validate_statements(&ctx, &overlay.value.statements, &mut Vec::new())?;
        }
    }
    let mut root = if let Some(initial) = initial {
        let ctx = Context {
            schema,
            source: &schema.source,
            overlay: None,
        };
        materialize(&ctx, schema.root, initial.clone(), Span::default()).map_err(|report| {
            let message = report
                .diagnostics
                .first()
                .map_or_else(|| "invalid value".to_owned(), |value| value.message.clone());
            schema.report(
                Span::default(),
                format!("initial validation failed: {message}"),
            )
        })?
    } else {
        schema
            .canonical(schema.root)
            .map_err(|message| schema.report(Span::default(), message))?
    };
    for document in overlays {
        for overlay in &document.ast.overlays {
            let ctx = Context {
                schema,
                source: &document.source,
                overlay: Some(&overlay.value.spelling),
            };
            execute_statements(&ctx, &overlay.value, &mut root, &mut Vec::new())?;
        }
    }
    schema
        .validate_complete(schema.root, &root)
        .map_err(|message| {
            schema.report(
                Span::default(),
                format!("final validation failed: {message}"),
            )
        })?;
    Ok(root)
}

fn validate_statements(
    ctx: &Context<'_>,
    statements: &[Node<Statement>],
    scopes: &mut Scopes,
) -> Result<(), ErrorReport> {
    for statement in statements {
        match &statement.value {
            Statement::Action(kind, path, expression) => {
                let ty = path_type(ctx, ctx.schema.root, path, scopes, PathUse::Target)?;
                if !matches!(kind, ActionKind::Reset) {
                    validate_expr(
                        ctx,
                        expression.as_ref().expect("action expression"),
                        Some(ty),
                        scopes,
                    )?;
                }
            }
            Statement::If(branches, otherwise) => {
                for (condition, body) in branches {
                    let kind = validate_expr(ctx, condition, None, scopes)?;
                    if !kind.truthy() {
                        return ctx.fail(
                            condition.span,
                            "validation",
                            "condition must be bool, int, or float",
                        );
                    }
                    validate_statements(ctx, body, scopes)?;
                }
                if let Some(body) = otherwise {
                    validate_statements(ctx, body, scopes)?;
                }
            }
            Statement::For(name, spelling, iterable, body) => {
                if scopes.iter().rev().any(|scope| scope.contains_key(name)) {
                    return ctx.fail(
                        statement.span,
                        "validation",
                        format!("active loop variable `{spelling}` is not distinct"),
                    );
                }
                let kind = validate_expr(ctx, iterable, None, scopes)?;
                let variable =
                    match kind {
                        StaticType::Int => VarType::Int,
                        StaticType::List => VarType::Int,
                        StaticType::Map => VarType::String,
                        _ => return ctx.fail(
                            iterable.span,
                            "validation",
                            "loop iterable must statically resolve to exactly int, list, or map",
                        ),
                    };
                scopes.push(HashMap::from([(name.clone(), variable)]));
                validate_statements(ctx, body, scopes)?;
                scopes.pop();
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StaticType {
    String,
    Int,
    Float,
    Bool,
    Object,
    Map,
    List,
    Tuple,
    Any,
    Union,
}
impl StaticType {
    fn truthy(self) -> bool {
        matches!(self, Self::Bool | Self::Int | Self::Float | Self::Any)
    }
}

fn static_type(schema: &Schema, id: TypeId) -> Result<StaticType, ErrorReport> {
    Ok(match schema.node(id)? {
        TypeNode::String | TypeNode::Literal(Value::String(_)) => StaticType::String,
        TypeNode::Int | TypeNode::Literal(Value::Int(_)) => StaticType::Int,
        TypeNode::Float | TypeNode::Literal(Value::Float(_)) => StaticType::Float,
        TypeNode::Bool | TypeNode::Literal(Value::Bool(_)) => StaticType::Bool,
        TypeNode::Object(_) | TypeNode::Tagged { .. } => StaticType::Object,
        TypeNode::Map(_) => StaticType::Map,
        TypeNode::List { .. } => StaticType::List,
        TypeNode::Tuple(_) => StaticType::Tuple,
        TypeNode::Any => StaticType::Any,
        TypeNode::Union(branches) => {
            let mut kinds = branches.iter().map(|branch| static_type(schema, *branch));
            let Some(first) = kinds.next() else {
                return Ok(StaticType::Union);
            };
            let first = first?;
            if kinds.all(|kind| kind == Ok(first)) {
                first
            } else {
                StaticType::Union
            }
        }
        _ => unreachable!(),
    })
}

fn validate_expr(
    ctx: &Context<'_>,
    expression: &Expr,
    expected: Option<TypeId>,
    scopes: &Scopes,
) -> Result<StaticType, ErrorReport> {
    let kind = match &expression.kind {
        ExprKind::Literal(value) => value.static_type(),
        ExprKind::Path(path) => static_type(
            ctx.schema,
            path_type(ctx, ctx.schema.root, path, scopes, PathUse::Read)?,
        )?,
        ExprKind::Variable(name) => match find_scope(scopes, name) {
            Some(VarType::Int) => StaticType::Int,
            Some(VarType::String) => StaticType::String,
            None => {
                return ctx.fail(
                    expression.span,
                    "validation",
                    format!("bare name `{name}` is not an in-scope loop variable"),
                );
            }
        },
        ExprKind::Object(entries) => {
            check_overlap(ctx, entries)?;
            StaticType::Object
        }
        ExprKind::List(items) => {
            for item in items {
                validate_expr(ctx, item, None, scopes)?;
            }
            StaticType::List
        }
        ExprKind::Tuple(items) => {
            for item in items {
                validate_expr(ctx, item, None, scopes)?;
            }
            StaticType::Tuple
        }
        ExprKind::Unary(op, value) => {
            let kind = validate_expr(ctx, value, None, scopes)?;
            match op {
                UnaryOp::Not if kind.truthy() => StaticType::Bool,
                UnaryOp::Negate
                    if matches!(kind, StaticType::Int | StaticType::Float | StaticType::Any) =>
                {
                    kind
                }
                UnaryOp::Not => {
                    return ctx.fail(
                        expression.span,
                        "validation",
                        "`not` requires bool or number",
                    );
                }
                UnaryOp::Negate => {
                    return ctx.fail(expression.span, "validation", "unary `-` requires a number");
                }
            }
        }
        ExprKind::Binary(op, left, right) => {
            let left = validate_expr(ctx, left, None, scopes)?;
            let right = validate_expr(ctx, right, None, scopes)?;
            validate_binary(ctx, expression.span, *op, left, right)?
        }
    };
    if let Some(expected) = expected {
        let expected_kind = static_type(ctx.schema, expected)?;
        if !compatible(kind, expected_kind) {
            return ctx.fail(
                expression.span,
                "validation",
                format!(
                    "{} expression cannot be assigned to {}",
                    kind.name(),
                    ctx.schema.type_name(expected)
                ),
            );
        }
        if let ExprKind::Literal(value) = &expression.kind {
            ctx.schema
                .validate_value(expected, value, true)
                .map_err(|message| {
                    ctx.fail::<()>(expression.span, "validation", message)
                        .unwrap_err()
                })?;
        }
        validate_literal_shape(ctx, expression, expected, scopes)?;
    }
    Ok(kind)
}

fn validate_literal_shape(
    ctx: &Context<'_>,
    expression: &Expr,
    expected: TypeId,
    scopes: &Scopes,
) -> Result<(), ErrorReport> {
    match (&expression.kind, ctx.schema.node(expected)?) {
        (ExprKind::Object(entries), TypeNode::Object(fields)) => {
            for (path, value) in entries {
                let ty = relative_type(ctx, fields, path, scopes)?;
                validate_expr(ctx, value, Some(ty), scopes)?;
            }
        }
        (ExprKind::Object(entries), TypeNode::Map(inner)) => {
            for (path, value) in entries {
                let ty = dotted_tail_type(ctx, *inner, &path.segments[1..], scopes)?;
                validate_expr(ctx, value, Some(ty), scopes)?;
            }
        }
        (ExprKind::List(items), TypeNode::List { item, key }) => {
            let mut identities = HashSet::new();
            for value in items {
                if let Some(key) = key {
                    let ExprKind::Object(entries) = &value.kind else {
                        return ctx.fail(
                            value.span,
                            "validation",
                            "keyed-list item must be an object literal",
                        );
                    };
                    let identity = entries.iter().find(|(path, _)| {
                        path.segments.len() == 1
                            && !path.segments[0].quoted
                            && path.segments[0].value == *key
                    });
                    let Some((_, identity_expr)) = identity else {
                        return ctx.fail(
                            value.span,
                            "validation",
                            format!("keyed-list item must explicitly contain `{key}`"),
                        );
                    };
                    if let ExprKind::Literal(identity) = &identity_expr.kind {
                        let encoded = identity_key(identity).map_err(|message| {
                            ctx.fail::<()>(identity_expr.span, "validation", message)
                                .unwrap_err()
                        })?;
                        if !identities.insert(encoded) {
                            return ctx.fail(
                                identity_expr.span,
                                "validation",
                                "duplicate keyed-list identity",
                            );
                        }
                    }
                }
                validate_expr(ctx, value, Some(*item), scopes)?;
            }
        }
        (ExprKind::Tuple(items), TypeNode::Tuple(types)) => {
            if items.len() != types.len() {
                return ctx.fail(
                    expression.span,
                    "validation",
                    format!("tuple requires {} elements", types.len()),
                );
            }
            for (value, ty) in items.iter().zip(types) {
                validate_expr(ctx, value, Some(*ty), scopes)?;
            }
        }
        (ExprKind::Object(entries), TypeNode::Tagged { tag, variants }) => {
            if !entries.iter().any(|(path, _)| {
                path.segments.len() == 1
                    && !path.segments[0].quoted
                    && path.segments[0].value == *tag
            }) {
                return ctx.fail(
                    expression.span,
                    "validation",
                    format!("tagged value must explicitly contain `{tag}`"),
                );
            }
            for (path, value) in entries {
                let first = &path.segments[0];
                if first.value == *tag {
                    if path.segments.len() != 1 {
                        return ctx.fail(
                            path.span,
                            "validation",
                            "tag discriminator cannot be traversed",
                        );
                    }
                    if !matches!(&value.kind, ExprKind::Literal(Value::String(_))) {
                        return ctx.fail(
                            value.span,
                            "validation",
                            "tag discriminator must be a string",
                        );
                    }
                    continue;
                }
                let field = variants
                    .iter()
                    .find_map(|variant| {
                        variant
                            .fields
                            .iter()
                            .find(|field| field.name == first.value)
                    })
                    .ok_or_else(|| {
                        ctx.fail::<()>(
                            first.span,
                            "validation",
                            format!("unknown tagged field `{}`", first.value),
                        )
                        .unwrap_err()
                    })?;
                let ty = dotted_tail_type(ctx, field.ty, &path.segments[1..], scopes)?;
                validate_expr(ctx, value, Some(ty), scopes)?;
            }
        }
        (_, TypeNode::Union(branches)) => {
            let mut last_error = None;
            for branch in branches {
                match validate_literal_shape(ctx, expression, *branch, scopes) {
                    Ok(()) => return Ok(()),
                    Err(error) => last_error = Some(error),
                }
            }
            if let Some(error) = last_error {
                return Err(error);
            }
        }
        _ => {}
    }
    Ok(())
}

fn compatible(actual: StaticType, expected: StaticType) -> bool {
    actual == expected
        || matches!(actual, StaticType::Any)
        || matches!(expected, StaticType::Any | StaticType::Union)
        || matches!((actual, expected), (StaticType::Object, StaticType::Map))
}
impl StaticType {
    fn name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::Object => "object",
            Self::Map => "map",
            Self::List => "list",
            Self::Tuple => "tuple",
            Self::Any => "any",
            Self::Union => "union",
        }
    }
}

fn validate_binary(
    ctx: &Context<'_>,
    span: Span,
    op: BinaryOp,
    left: StaticType,
    right: StaticType,
) -> Result<StaticType, ErrorReport> {
    use BinaryOp::*;
    if matches!(left, StaticType::Any) || matches!(right, StaticType::Any) {
        return Ok(
            if matches!(
                op,
                Or | And | Equal | Less | Greater | LessEqual | GreaterEqual
            ) {
                StaticType::Bool
            } else {
                StaticType::Any
            },
        );
    }
    match op {
        Or | And if left.truthy() && right.truthy() => Ok(StaticType::Bool),
        Equal
            if left == right
                && matches!(
                    left,
                    StaticType::String | StaticType::Int | StaticType::Float | StaticType::Bool
                ) =>
        {
            Ok(StaticType::Bool)
        }
        Less | Greater | LessEqual | GreaterEqual
            if left == right && matches!(left, StaticType::Int | StaticType::Float) =>
        {
            Ok(StaticType::Bool)
        }
        Add if left == StaticType::String && right == StaticType::String => Ok(StaticType::String),
        Add | Subtract | Multiply | Divide
            if matches!(left, StaticType::Int | StaticType::Float)
                && matches!(right, StaticType::Int | StaticType::Float) =>
        {
            Ok(if left == StaticType::Float || right == StaticType::Float {
                StaticType::Float
            } else {
                StaticType::Int
            })
        }
        _ => ctx.fail(
            span,
            "validation",
            "operator operand types are incompatible",
        ),
    }
}

fn path_type(
    ctx: &Context<'_>,
    ty: TypeId,
    path: &Path,
    scopes: &Scopes,
    path_use: PathUse,
) -> Result<TypeId, ErrorReport> {
    path_type_segments(ctx, ty, &path.segments, scopes, path_use, true)
}

fn path_type_segments(
    ctx: &Context<'_>,
    ty: TypeId,
    segments: &[PathSegment],
    scopes: &Scopes,
    path_use: PathUse,
    allow_map_position: bool,
) -> Result<TypeId, ErrorReport> {
    let Some((segment, rest)) = segments.split_first() else {
        return Ok(ty);
    };
    let variable = (!segment.quoted)
        .then(|| find_scope(scopes, &segment.value))
        .flatten();
    match ctx.schema.node(ty)? {
        TypeNode::Object(fields) => {
            if segment.quoted {
                return ctx.fail(
                    segment.span,
                    "validation",
                    "quoted path segment cannot select a fixed-object field",
                );
            }
            if variable.is_some() {
                return ctx.fail(
                    segment.span,
                    "validation",
                    "loop-variable segment cannot select a fixed-object field",
                );
            }
            let field = fields
                .iter()
                .find(|field| field.name == segment.value)
                .ok_or_else(|| {
                    ctx.fail::<()>(
                        segment.span,
                        "validation",
                        format!("unknown field `{}`", segment.value),
                    )
                    .unwrap_err()
                })?;
            path_type_segments(ctx, field.ty, rest, scopes, path_use, allow_map_position)
        }
        TypeNode::Map(inner) => {
            let numeric = !segment.quoted
                && (matches!(variable, Some(VarType::Int))
                    || (variable.is_none() && segment.value.parse::<usize>().is_ok()));
            if numeric && !allow_map_position {
                return ctx.fail(
                    segment.span,
                    "validation",
                    "positional map access requires a statically known map",
                );
            }
            let positional = allow_map_position && numeric;
            if positional {
                if path_use == PathUse::Target {
                    return ctx.fail(
                        segment.span,
                        "validation",
                        "positional map access is read-only",
                    );
                }
                let Some((selector, tail)) = rest.split_first() else {
                    return ctx.fail(
                        segment.span,
                        "validation",
                        "map index must be followed by `key` or `value`",
                    );
                };
                if selector.quoted {
                    return ctx.fail(
                        selector.span,
                        "validation",
                        "map index must be followed by `key` or `value`",
                    );
                }
                let selected_ty = match selector.value.as_str() {
                    "key" => string_type(ctx.schema)?,
                    "value" => *inner,
                    _ => {
                        return ctx.fail(
                            selector.span,
                            "validation",
                            "map index must be followed by `key` or `value`",
                        );
                    }
                };
                return path_type_segments(
                    ctx,
                    selected_ty,
                    tail,
                    scopes,
                    path_use,
                    allow_map_position,
                );
            }
            if matches!(variable, Some(VarType::Int)) {
                return ctx.fail(
                    segment.span,
                    "validation",
                    "integer loop variable cannot select a map key",
                );
            }
            path_type_segments(ctx, *inner, rest, scopes, path_use, allow_map_position)
        }
        TypeNode::List { item, .. } => {
            if segment.quoted
                || (!matches!(variable, Some(VarType::Int))
                    && segment.value.parse::<usize>().is_err())
            {
                return ctx.fail(
                    segment.span,
                    "validation",
                    "list path requires an integer index",
                );
            }
            path_type_segments(ctx, *item, rest, scopes, path_use, allow_map_position)
        }
        TypeNode::Tuple(items) => {
            if segment.quoted {
                return ctx.fail(
                    segment.span,
                    "validation",
                    "tuple path requires an integer index",
                );
            }
            let item = if matches!(variable, Some(VarType::Int)) {
                items.first().copied().ok_or_else(|| {
                    ctx.fail::<()>(segment.span, "validation", "cannot index an empty tuple")
                        .unwrap_err()
                })?
            } else {
                let index = segment.value.parse::<usize>().map_err(|_| {
                    ctx.fail::<()>(
                        segment.span,
                        "validation",
                        "tuple path requires an integer index",
                    )
                    .unwrap_err()
                })?;
                *items.get(index).ok_or_else(|| {
                    ctx.fail::<()>(segment.span, "validation", "tuple index is out of range")
                        .unwrap_err()
                })?
            };
            path_type_segments(ctx, item, rest, scopes, path_use, allow_map_position)
        }
        TypeNode::Tagged { tag, variants } => {
            if segment.quoted {
                return ctx.fail(
                    segment.span,
                    "validation",
                    "quoted path segment cannot select a tagged-object field",
                );
            }
            let field_ty = if segment.value == *tag {
                string_type(ctx.schema)?
            } else {
                variants
                    .iter()
                    .find_map(|variant| {
                        variant
                            .fields
                            .iter()
                            .find(|field| field.name == segment.value)
                    })
                    .map(|field| field.ty)
                    .ok_or_else(|| {
                        ctx.fail::<()>(
                            segment.span,
                            "validation",
                            format!("unknown tagged field `{}`", segment.value),
                        )
                        .unwrap_err()
                    })?
            };
            path_type_segments(ctx, field_ty, rest, scopes, path_use, allow_map_position)
        }
        TypeNode::Union(branches) => {
            for branch in branches {
                if let Ok(value) =
                    path_type_segments(ctx, *branch, segments, scopes, path_use, false)
                {
                    return Ok(value);
                }
            }
            ctx.fail(
                segment.span,
                "validation",
                "path is invalid for every union branch",
            )
        }
        TypeNode::Any => Ok(ty),
        _ => ctx.fail(
            segment.span,
            "validation",
            format!("cannot traverse {}", ctx.schema.type_name(ty)),
        ),
    }
}

fn string_type(schema: &Schema) -> Result<TypeId, ErrorReport> {
    schema
        .types
        .iter()
        .position(|node| matches!(node, TypeNode::String))
        .ok_or_else(|| schema.report(Span::default(), "internal string type unavailable"))
}

fn relative_type(
    ctx: &Context<'_>,
    fields: &[Field],
    path: &Path,
    scopes: &Scopes,
) -> Result<TypeId, ErrorReport> {
    let first = &path.segments[0];
    if first.quoted {
        return ctx.fail(
            first.span,
            "validation",
            "quoted key cannot select a fixed-object field",
        );
    }
    let field = fields
        .iter()
        .find(|field| field.name == first.value)
        .ok_or_else(|| {
            ctx.fail::<()>(
                first.span,
                "validation",
                format!("unknown field `{}`", first.value),
            )
            .unwrap_err()
        })?;
    if path.segments.len() == 1 {
        return Ok(field.ty);
    }
    dotted_tail_type(ctx, field.ty, &path.segments[1..], scopes)
}

fn dotted_tail_type(
    ctx: &Context<'_>,
    mut ty: TypeId,
    segments: &[PathSegment],
    scopes: &Scopes,
) -> Result<TypeId, ErrorReport> {
    for segment in segments {
        if matches!(ctx.schema.node(ty)?, TypeNode::List { .. }) {
            return ctx.fail(
                segment.span,
                "validation",
                "dotted shorthand must not traverse a list",
            );
        }
        ty = path_type(
            ctx,
            ty,
            &Path {
                segments: vec![segment.clone()],
                span: segment.span,
            },
            scopes,
            PathUse::Target,
        )?;
    }
    Ok(ty)
}

fn check_overlap(ctx: &Context<'_>, entries: &[(Path, Expr)]) -> Result<(), ErrorReport> {
    for (index, (left, _)) in entries.iter().enumerate() {
        for (right, _) in &entries[index + 1..] {
            let common = left
                .segments
                .iter()
                .zip(&right.segments)
                .take_while(|(a, b)| a.value == b.value)
                .count();
            if common == left.segments.len().min(right.segments.len()) {
                return ctx.fail(
                    right.span,
                    "validation",
                    "dotted fields duplicate or overlap within one literal",
                );
            }
        }
    }
    Ok(())
}

fn find_scope(scopes: &Scopes, name: &str) -> Option<VarType> {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(name).copied())
}
fn find_var<'a>(vars: &'a Vars, name: &str) -> Option<&'a Value> {
    vars.iter().rev().find_map(|scope| scope.get(name))
}

fn execute_statements(
    ctx: &Context<'_>,
    overlay: &Overlay,
    root: &mut Value,
    vars: &mut Vars,
) -> Result<(), ErrorReport> {
    for statement in &overlay.statements {
        execute_statement(ctx, statement, root, vars)?;
    }
    Ok(())
}

fn execute_body(
    ctx: &Context<'_>,
    body: &[Node<Statement>],
    root: &mut Value,
    vars: &mut Vars,
) -> Result<(), ErrorReport> {
    for statement in body {
        execute_statement(ctx, statement, root, vars)?;
    }
    Ok(())
}

fn execute_statement(
    ctx: &Context<'_>,
    statement: &Node<Statement>,
    root: &mut Value,
    vars: &mut Vars,
) -> Result<(), ErrorReport> {
    match &statement.value {
        Statement::Action(kind, path, expression) => {
            let snapshot = root.clone();
            let segments = substitute_path(ctx, path, vars)?;
            if matches!(kind, ActionKind::Reset) {
                reset_at(ctx, ctx.schema.root, root, &segments, 0, path.span)?;
            } else {
                let incoming = eval_expr(
                    ctx,
                    expression.as_ref().expect("expression"),
                    &snapshot,
                    vars,
                )?;
                apply_at(
                    ctx,
                    ctx.schema.root,
                    root,
                    &segments,
                    0,
                    *kind,
                    incoming,
                    path.span,
                )?;
            }
        }
        Statement::If(branches, otherwise) => {
            for (condition, body) in branches {
                let snapshot = root.clone();
                if truthy(
                    ctx,
                    condition.span,
                    &eval_expr(ctx, condition, &snapshot, vars)?,
                )? {
                    return execute_body(ctx, body, root, vars);
                }
            }
            if let Some(body) = otherwise {
                execute_body(ctx, body, root, vars)?;
            }
        }
        Statement::For(name, _, iterable, body) => {
            let snapshot = root.clone();
            let value = eval_expr(ctx, iterable, &snapshot, vars)?;
            let values: Vec<Value> = match value {
                Value::Int(value) if value >= 0 => (0..value).map(Value::Int).collect(),
                Value::Int(_) => {
                    return ctx.fail(iterable.span, "evaluation", "negative loop bound");
                }
                Value::List(values) => (0..values.len())
                    .map(|index| Value::Int(index as i64))
                    .collect(),
                Value::Object(values) => values.keys().cloned().map(Value::String).collect(),
                _ => {
                    return ctx.fail(
                        iterable.span,
                        "evaluation",
                        "loop iterable is not int, list, or map",
                    );
                }
            };
            for value in values {
                vars.push(HashMap::from([(name.clone(), value)]));
                execute_body(ctx, body, root, vars)?;
                vars.pop();
            }
        }
    }
    Ok(())
}

fn substitute_path(
    ctx: &Context<'_>,
    path: &Path,
    vars: &Vars,
) -> Result<Vec<String>, ErrorReport> {
    substitute_segments(ctx, &path.segments, vars)
}

fn substitute_segments(
    ctx: &Context<'_>,
    segments: &[PathSegment],
    vars: &Vars,
) -> Result<Vec<String>, ErrorReport> {
    segments
        .iter()
        .map(|segment| substitute_segment(ctx, segment, vars))
        .collect()
}

fn substitute_segment(
    ctx: &Context<'_>,
    segment: &PathSegment,
    vars: &Vars,
) -> Result<String, ErrorReport> {
    if segment.quoted {
        return Ok(segment.value.clone());
    }
    match find_var(vars, &segment.value) {
        Some(Value::Int(value)) if *value >= 0 => Ok(value.to_string()),
        Some(Value::String(value)) => Ok(value.to_ascii_lowercase()),
        Some(_) => ctx.fail(
            segment.span,
            "evaluation",
            "invalid loop-variable path segment",
        ),
        None => Ok(segment.value.clone()),
    }
}

fn eval_expr(
    ctx: &Context<'_>,
    expression: &Expr,
    root: &Value,
    vars: &Vars,
) -> Result<Value, ErrorReport> {
    match &expression.kind {
        ExprKind::Literal(value) => Ok(value.clone()),
        ExprKind::Variable(name) => find_var(vars, name).cloned().ok_or_else(|| {
            ctx.fail::<()>(
                expression.span,
                "evaluation",
                format!("unknown loop variable `{name}`"),
            )
            .unwrap_err()
        }),
        ExprKind::Path(path) => {
            read_path(ctx, ctx.schema.root, root, &path.segments, vars, path.span)
        }
        ExprKind::Object(entries) => {
            let mut object = IndexMap::new();
            for (path, value) in entries {
                let segments = substitute_path(ctx, path, vars)?;
                let value = eval_expr(ctx, value, root, vars)?;
                insert_dotted(ctx, &mut object, &segments, value, path.span)?;
            }
            Ok(Value::Object(object))
        }
        ExprKind::List(items) => {
            let mut values = Vec::new();
            for item in items {
                values.push(eval_expr(ctx, item, root, vars)?);
            }
            Ok(Value::List(values))
        }
        ExprKind::Tuple(items) => {
            let mut values = Vec::new();
            for item in items {
                values.push(eval_expr(ctx, item, root, vars)?);
            }
            Ok(Value::Tuple(values))
        }
        ExprKind::Unary(UnaryOp::Not, value) => Ok(Value::Bool(!truthy(
            ctx,
            expression.span,
            &eval_expr(ctx, value, root, vars)?,
        )?)),
        ExprKind::Unary(UnaryOp::Negate, value) => match eval_expr(ctx, value, root, vars)? {
            Value::Int(value) => value.checked_neg().map(Value::Int).ok_or_else(|| {
                ctx.fail::<()>(expression.span, "evaluation", "integer overflow")
                    .unwrap_err()
            }),
            Value::Float(value) => finite(ctx, expression.span, -value),
            _ => ctx.fail(expression.span, "evaluation", "unary `-` requires a number"),
        },
        ExprKind::Binary(BinaryOp::And, left, right) => {
            let left = eval_expr(ctx, left, root, vars)?;
            if !truthy(ctx, expression.span, &left)? {
                Ok(Value::Bool(false))
            } else {
                Ok(Value::Bool(truthy(
                    ctx,
                    expression.span,
                    &eval_expr(ctx, right, root, vars)?,
                )?))
            }
        }
        ExprKind::Binary(BinaryOp::Or, left, right) => {
            let left = eval_expr(ctx, left, root, vars)?;
            if truthy(ctx, expression.span, &left)? {
                Ok(Value::Bool(true))
            } else {
                Ok(Value::Bool(truthy(
                    ctx,
                    expression.span,
                    &eval_expr(ctx, right, root, vars)?,
                )?))
            }
        }
        ExprKind::Binary(op, left, right) => {
            let left = eval_expr(ctx, left, root, vars)?;
            let right = eval_expr(ctx, right, root, vars)?;
            eval_binary(ctx, expression.span, *op, left, right)
        }
    }
}

fn eval_binary(
    ctx: &Context<'_>,
    span: Span,
    op: BinaryOp,
    left: Value,
    right: Value,
) -> Result<Value, ErrorReport> {
    use BinaryOp::*;
    match (op, left, right) {
        (Equal, Value::String(a), Value::String(b)) => Ok(Value::Bool(a == b)),
        (Equal, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
        (Equal, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a == b)),
        (Equal, Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
        (Less, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (Greater, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
        (LessEqual, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
        (GreaterEqual, Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
        (Less, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
        (Greater, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
        (LessEqual, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
        (GreaterEqual, Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
        (Add, Value::String(a), Value::String(b)) => Ok(Value::String(a + &b)),
        (op @ (Add | Subtract | Multiply | Divide), Value::Int(a), Value::Int(b)) => {
            let value = match op {
                Add => a.checked_add(b),
                Subtract => a.checked_sub(b),
                Multiply => a.checked_mul(b),
                Divide => a.checked_div(b),
                _ => unreachable!(),
            };
            value.map(Value::Int).ok_or_else(|| {
                ctx.fail::<()>(
                    span,
                    "evaluation",
                    if b == 0 && op == Divide {
                        "integer division by zero"
                    } else {
                        "integer overflow"
                    },
                )
                .unwrap_err()
            })
        }
        (op @ (Add | Subtract | Multiply | Divide), Value::Float(a), Value::Float(b)) => {
            finite(ctx, span, float_op(op, a, b))
        }
        (op @ (Add | Subtract | Multiply | Divide), Value::Int(a), Value::Float(b)) => {
            finite(ctx, span, float_op(op, a as f64, b))
        }
        (op @ (Add | Subtract | Multiply | Divide), Value::Float(a), Value::Int(b)) => {
            finite(ctx, span, float_op(op, a, b as f64))
        }
        _ => ctx.fail(
            span,
            "evaluation",
            "operator operand types are incompatible",
        ),
    }
}
fn float_op(op: BinaryOp, a: f64, b: f64) -> f64 {
    match op {
        BinaryOp::Add => a + b,
        BinaryOp::Subtract => a - b,
        BinaryOp::Multiply => a * b,
        BinaryOp::Divide => a / b,
        _ => unreachable!(),
    }
}
fn finite(ctx: &Context<'_>, span: Span, value: f64) -> Result<Value, ErrorReport> {
    if value.is_finite() {
        Ok(Value::Float(value))
    } else {
        ctx.fail(
            span,
            "evaluation",
            "floating-point operation produced a non-finite value",
        )
    }
}
fn truthy(ctx: &Context<'_>, span: Span, value: &Value) -> Result<bool, ErrorReport> {
    match value {
        Value::Bool(v) => Ok(*v),
        Value::Int(v) => Ok(*v != 0),
        Value::Float(v) => Ok(*v != 0.0),
        _ => ctx.fail(
            span,
            "evaluation",
            "truth operation requires bool or number",
        ),
    }
}

fn read_path(
    ctx: &Context<'_>,
    ty: TypeId,
    value: &Value,
    segments: &[PathSegment],
    vars: &Vars,
    span: Span,
) -> Result<Value, ErrorReport> {
    read_path_inner(ctx, ty, value, segments, vars, span, true)
}

#[allow(clippy::too_many_arguments)]
fn read_path_inner(
    ctx: &Context<'_>,
    ty: TypeId,
    value: &Value,
    segments: &[PathSegment],
    vars: &Vars,
    span: Span,
    allow_map_position: bool,
) -> Result<Value, ErrorReport> {
    let Some((segment_node, rest)) = segments.split_first() else {
        return Ok(value.clone());
    };
    match ctx.schema.node(ty)? {
        TypeNode::Object(fields) => {
            let segment = substitute_segment(ctx, segment_node, vars)?;
            let field = fields
                .iter()
                .find(|field| field.name == segment)
                .ok_or_else(|| {
                    ctx.fail::<()>(
                        segment_node.span,
                        "evaluation",
                        format!("unknown field `{segment}`"),
                    )
                    .unwrap_err()
                })?;
            let Value::Object(values) = value else {
                return ctx.fail(span, "evaluation", "expected object while reading path");
            };
            let child = values.get(&segment).ok_or_else(|| {
                ctx.fail::<()>(
                    segment_node.span,
                    "evaluation",
                    format!("missing path segment `{segment}`"),
                )
                .unwrap_err()
            })?;
            read_path_inner(ctx, field.ty, child, rest, vars, span, allow_map_position)
        }
        TypeNode::Map(inner) => {
            let Value::Object(values) = value else {
                return ctx.fail(span, "evaluation", "expected map while reading path");
            };
            let positional_index = if allow_map_position && !segment_node.quoted {
                match find_var(vars, &segment_node.value) {
                    Some(Value::Int(index)) if *index >= 0 => Some(*index as usize),
                    Some(Value::Int(_)) => {
                        return ctx.fail(
                            segment_node.span,
                            "evaluation",
                            "invalid loop-variable path segment",
                        );
                    }
                    Some(Value::String(_)) => None,
                    Some(_) => {
                        return ctx.fail(
                            segment_node.span,
                            "evaluation",
                            "invalid loop-variable path segment",
                        );
                    }
                    None => segment_node.value.parse::<usize>().ok(),
                }
            } else {
                None
            };
            if let Some(index) = positional_index {
                let Some((selector, tail)) = rest.split_first() else {
                    return ctx.fail(
                        segment_node.span,
                        "evaluation",
                        "map index must be followed by `key` or `value`",
                    );
                };
                if selector.quoted {
                    return ctx.fail(
                        selector.span,
                        "evaluation",
                        "map index must be followed by `key` or `value`",
                    );
                }
                let (key, item) = values.get_index(index).ok_or_else(|| {
                    ctx.fail::<()>(segment_node.span, "evaluation", "map index is out of range")
                        .unwrap_err()
                })?;
                return match selector.value.as_str() {
                    "key" => {
                        let key = Value::String(key.clone());
                        read_path_inner(
                            ctx,
                            string_type(ctx.schema)?,
                            &key,
                            tail,
                            vars,
                            span,
                            allow_map_position,
                        )
                    }
                    "value" => {
                        read_path_inner(ctx, *inner, item, tail, vars, span, allow_map_position)
                    }
                    _ => ctx.fail(
                        selector.span,
                        "evaluation",
                        "map index must be followed by `key` or `value`",
                    ),
                };
            }
            let segment = substitute_segment(ctx, segment_node, vars)?;
            let child = values.get(&segment).ok_or_else(|| {
                ctx.fail::<()>(
                    segment_node.span,
                    "evaluation",
                    format!("missing path segment `{segment}`"),
                )
                .unwrap_err()
            })?;
            read_path_inner(ctx, *inner, child, rest, vars, span, allow_map_position)
        }
        TypeNode::List { item, .. } => {
            let Value::List(values) = value else {
                return ctx.fail(span, "evaluation", "expected list while reading path");
            };
            let segment = substitute_segment(ctx, segment_node, vars)?;
            let index = parse_index(ctx, &segment, segment_node.span)?;
            let child = values.get(index).ok_or_else(|| {
                ctx.fail::<()>(segment_node.span, "evaluation", "index is out of range")
                    .unwrap_err()
            })?;
            read_path_inner(ctx, *item, child, rest, vars, span, allow_map_position)
        }
        TypeNode::Tuple(items) => {
            let Value::Tuple(values) = value else {
                return ctx.fail(span, "evaluation", "expected tuple while reading path");
            };
            let segment = substitute_segment(ctx, segment_node, vars)?;
            let index = parse_index(ctx, &segment, segment_node.span)?;
            let item = *items.get(index).ok_or_else(|| {
                ctx.fail::<()>(
                    segment_node.span,
                    "evaluation",
                    "tuple index is out of range",
                )
                .unwrap_err()
            })?;
            let child = values.get(index).ok_or_else(|| {
                ctx.fail::<()>(
                    segment_node.span,
                    "evaluation",
                    "tuple index is out of range",
                )
                .unwrap_err()
            })?;
            read_path_inner(ctx, item, child, rest, vars, span, allow_map_position)
        }
        TypeNode::Tagged { tag, variants } => {
            let segment = substitute_segment(ctx, segment_node, vars)?;
            let Value::Object(values) = value else {
                return ctx.fail(
                    span,
                    "evaluation",
                    "expected tagged object while reading path",
                );
            };
            let field_ty = if segment == *tag {
                string_type(ctx.schema)?
            } else {
                let discriminator = match values.get(tag) {
                    Some(Value::String(value)) => value,
                    _ => return ctx.fail(span, "evaluation", "missing tagged discriminator"),
                };
                variants
                    .iter()
                    .find(|variant| &variant.name == discriminator)
                    .and_then(|variant| variant.fields.iter().find(|field| field.name == segment))
                    .map(|field| field.ty)
                    .ok_or_else(|| {
                        ctx.fail::<()>(
                            segment_node.span,
                            "evaluation",
                            format!("field `{segment}` is not in the active variant"),
                        )
                        .unwrap_err()
                    })?
            };
            let child = values.get(&segment).ok_or_else(|| {
                ctx.fail::<()>(
                    segment_node.span,
                    "evaluation",
                    format!("missing path segment `{segment}`"),
                )
                .unwrap_err()
            })?;
            read_path_inner(ctx, field_ty, child, rest, vars, span, allow_map_position)
        }
        TypeNode::Union(branches) => {
            let branch = select_existing(ctx.schema, branches, value).ok_or_else(|| {
                ctx.fail::<()>(span, "evaluation", "value matches no union branch")
                    .unwrap_err()
            })?;
            read_path_inner(ctx, branch, value, segments, vars, span, false)
        }
        TypeNode::Any => {
            let segments = substitute_segments(ctx, segments, vars)?;
            read_dynamic_path(ctx, value, &segments, span).cloned()
        }
        _ => ctx.fail(
            segment_node.span,
            "evaluation",
            "path traverses a primitive value",
        ),
    }
}

fn read_dynamic_path<'a>(
    ctx: &Context<'_>,
    mut value: &'a Value,
    segments: &[String],
    span: Span,
) -> Result<&'a Value, ErrorReport> {
    for segment in segments {
        value = match value {
            Value::Object(values) => values.get(segment).ok_or_else(|| {
                ctx.fail::<()>(
                    span,
                    "evaluation",
                    format!("missing path segment `{segment}`"),
                )
                .unwrap_err()
            })?,
            Value::List(values) | Value::Tuple(values) => {
                let index = segment.parse::<usize>().map_err(|_| {
                    ctx.fail::<()>(span, "evaluation", "collection path requires integer index")
                        .unwrap_err()
                })?;
                values.get(index).ok_or_else(|| {
                    ctx.fail::<()>(span, "evaluation", "index is out of range")
                        .unwrap_err()
                })?
            }
            _ => return ctx.fail(span, "evaluation", "path traverses a primitive value"),
        };
    }
    Ok(value)
}

fn insert_dotted(
    ctx: &Context<'_>,
    object: &mut IndexMap<String, Value>,
    segments: &[String],
    value: Value,
    span: Span,
) -> Result<(), ErrorReport> {
    let (first, rest) = segments.split_first().expect("relative path");
    if rest.is_empty() {
        object.insert(first.clone(), value);
        return Ok(());
    }
    let entry = object
        .entry(first.clone())
        .or_insert_with(|| Value::Object(IndexMap::new()));
    let Value::Object(nested) = entry else {
        return ctx.fail(
            span,
            "evaluation",
            "dotted shorthand overlaps a non-object value",
        );
    };
    insert_dotted(ctx, nested, rest, value, span)
}

#[allow(clippy::too_many_arguments)]
fn apply_at(
    ctx: &Context<'_>,
    ty: TypeId,
    target: &mut Value,
    segments: &[String],
    depth: usize,
    kind: ActionKind,
    incoming: Value,
    span: Span,
) -> Result<(), ErrorReport> {
    if depth == segments.len() {
        let policy = if matches!(kind, ActionKind::Merge) {
            Policy::Merge
        } else {
            Policy::Hybrid
        };
        if matches!(kind, ActionKind::Set) {
            *target = fresh_for(ctx, ty, &incoming, span)?;
        }
        populate(ctx, ty, target, incoming, policy, span)?;
        ctx.schema
            .validate_complete(ty, target)
            .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
        return Ok(());
    }
    let segment = &segments[depth];
    let resolved = ctx
        .schema
        .resolved(ty)
        .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
    match ctx.schema.types[resolved].clone() {
        TypeNode::Object(fields) => {
            let field = fields
                .iter()
                .find(|field| &field.name == segment)
                .ok_or_else(|| {
                    ctx.fail::<()>(span, "evaluation", format!("unknown field `{segment}`"))
                        .unwrap_err()
                })?;
            let Value::Object(values) = target else {
                return ctx.fail(
                    span,
                    "evaluation",
                    "expected object while traversing target",
                );
            };
            if !values.contains_key(segment) {
                let value = ctx
                    .schema
                    .canonical(field.ty)
                    .unwrap_or_else(|_| Value::Object(IndexMap::new()));
                values.insert(segment.clone(), value);
            }
            apply_at(
                ctx,
                field.ty,
                values.get_mut(segment).expect("inserted"),
                segments,
                depth + 1,
                kind,
                incoming,
                span,
            )
        }
        TypeNode::Map(inner) => {
            let Value::Object(values) = target else {
                return ctx.fail(span, "evaluation", "expected map while traversing target");
            };
            if !values.contains_key(segment) {
                values.insert(
                    segment.clone(),
                    ctx.schema
                        .canonical(inner)
                        .unwrap_or_else(|_| incoming.clone()),
                );
            }
            apply_at(
                ctx,
                inner,
                values.get_mut(segment).expect("inserted"),
                segments,
                depth + 1,
                kind,
                incoming,
                span,
            )
        }
        TypeNode::List { item, .. } => {
            let Value::List(values) = target else {
                return ctx.fail(span, "evaluation", "expected list while traversing target");
            };
            let index = parse_index(ctx, segment, span)?;
            let value = values.get_mut(index).ok_or_else(|| {
                ctx.fail::<()>(span, "evaluation", "list index is out of range")
                    .unwrap_err()
            })?;
            apply_at(ctx, item, value, segments, depth + 1, kind, incoming, span)
        }
        TypeNode::Tuple(items) => {
            let Value::Tuple(values) = target else {
                return ctx.fail(span, "evaluation", "expected tuple while traversing target");
            };
            let index = parse_index(ctx, segment, span)?;
            let item = *items.get(index).ok_or_else(|| {
                ctx.fail::<()>(span, "evaluation", "tuple index is out of range")
                    .unwrap_err()
            })?;
            apply_at(
                ctx,
                item,
                &mut values[index],
                segments,
                depth + 1,
                kind,
                incoming,
                span,
            )
        }
        TypeNode::Tagged { tag, variants } => {
            let Value::Object(values) = target else {
                return ctx.fail(span, "evaluation", "expected tagged object");
            };
            let field_ty = if segment == &tag {
                return ctx.fail(span, "evaluation", "tag discriminator cannot be targeted without replacing the complete tagged value");
            } else {
                let discriminator = match values.get(&tag) {
                    Some(Value::String(value)) => value,
                    _ => return ctx.fail(span, "evaluation", "missing tagged discriminator"),
                };
                variants
                    .iter()
                    .find(|variant| &variant.name == discriminator)
                    .and_then(|variant| variant.fields.iter().find(|field| &field.name == segment))
                    .map(|field| field.ty)
                    .ok_or_else(|| {
                        ctx.fail::<()>(
                            span,
                            "evaluation",
                            format!("field `{segment}` is not in the active variant"),
                        )
                        .unwrap_err()
                    })?
            };
            if !values.contains_key(segment) {
                values.insert(
                    segment.clone(),
                    ctx.schema
                        .canonical(field_ty)
                        .unwrap_or_else(|_| incoming.clone()),
                );
            }
            apply_at(
                ctx,
                field_ty,
                values.get_mut(segment).expect("inserted"),
                segments,
                depth + 1,
                kind,
                incoming,
                span,
            )
        }
        TypeNode::Union(branches) => {
            let branch = select_existing(ctx.schema, &branches, target).unwrap_or(branches[0]);
            apply_at(ctx, branch, target, segments, depth, kind, incoming, span)
        }
        TypeNode::Any => apply_dynamic_at(ctx, target, segments, depth, incoming, span),
        _ => ctx.fail(span, "evaluation", "cannot traverse primitive target"),
    }
}

fn reset_at(
    ctx: &Context<'_>,
    ty: TypeId,
    target: &mut Value,
    segments: &[String],
    depth: usize,
    span: Span,
) -> Result<(), ErrorReport> {
    if depth == segments.len() {
        *target = ctx
            .schema
            .canonical(ty)
            .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
        return Ok(());
    }
    let segment = &segments[depth];
    let resolved = ctx
        .schema
        .resolved(ty)
        .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
    match ctx.schema.types[resolved].clone() {
        TypeNode::Object(fields) => {
            let field = fields
                .iter()
                .find(|field| &field.name == segment)
                .ok_or_else(|| {
                    ctx.fail::<()>(span, "evaluation", format!("unknown field `{segment}`"))
                        .unwrap_err()
                })?;
            let Value::Object(values) = target else {
                return ctx.fail(span, "evaluation", "expected object");
            };
            if depth + 1 == segments.len() {
                if field.optional {
                    values.shift_remove(segment);
                } else {
                    values.insert(
                        segment.clone(),
                        ctx.schema.canonical(field.ty).map_err(|message| {
                            ctx.fail::<()>(span, "evaluation", message).unwrap_err()
                        })?,
                    );
                }
                return Ok(());
            }
            let Some(value) = values.get_mut(segment) else {
                return Ok(());
            };
            reset_at(ctx, field.ty, value, segments, depth + 1, span)
        }
        TypeNode::Map(inner) => {
            let Value::Object(values) = target else {
                return ctx.fail(span, "evaluation", "expected map");
            };
            if depth + 1 == segments.len() {
                values.shift_remove(segment);
                return Ok(());
            }
            let Some(value) = values.get_mut(segment) else {
                return Ok(());
            };
            reset_at(ctx, inner, value, segments, depth + 1, span)
        }
        TypeNode::List { item, .. } => {
            let Value::List(values) = target else {
                return ctx.fail(span, "evaluation", "expected list");
            };
            let index = parse_index(ctx, segment, span)?;
            if index >= values.len() {
                return ctx.fail(span, "evaluation", "list index is out of range");
            }
            if depth + 1 == segments.len() {
                values.remove(index);
                Ok(())
            } else {
                reset_at(ctx, item, &mut values[index], segments, depth + 1, span)
            }
        }
        TypeNode::Tuple(items) => {
            let Value::Tuple(values) = target else {
                return ctx.fail(span, "evaluation", "expected tuple");
            };
            let index = parse_index(ctx, segment, span)?;
            let item = *items.get(index).ok_or_else(|| {
                ctx.fail::<()>(span, "evaluation", "tuple index is out of range")
                    .unwrap_err()
            })?;
            if depth + 1 == segments.len() {
                values[index] = ctx
                    .schema
                    .canonical(item)
                    .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
                Ok(())
            } else {
                reset_at(ctx, item, &mut values[index], segments, depth + 1, span)
            }
        }
        TypeNode::Union(branches) => {
            let branch = select_existing(ctx.schema, &branches, target).unwrap_or(branches[0]);
            reset_at(ctx, branch, target, segments, depth, span)
        }
        TypeNode::Tagged { tag, variants } => {
            let Value::Object(values) = target else {
                return ctx.fail(span, "evaluation", "expected tagged object");
            };
            let discriminator = match values.get(&tag) {
                Some(Value::String(value)) => value.clone(),
                _ => return ctx.fail(span, "evaluation", "missing tagged discriminator"),
            };
            let variant = variants
                .iter()
                .find(|variant| variant.name == discriminator)
                .ok_or_else(|| {
                    ctx.fail::<()>(span, "evaluation", "unknown tagged variant")
                        .unwrap_err()
                })?;
            let field = variant
                .fields
                .iter()
                .find(|field| &field.name == segment)
                .ok_or_else(|| {
                    ctx.fail::<()>(span, "evaluation", "field is not in active variant")
                        .unwrap_err()
                })?;
            if depth + 1 == segments.len() {
                if field.optional {
                    values.shift_remove(segment);
                } else {
                    values.insert(
                        segment.clone(),
                        ctx.schema.canonical(field.ty).map_err(|message| {
                            ctx.fail::<()>(span, "evaluation", message).unwrap_err()
                        })?,
                    );
                }
                Ok(())
            } else {
                let Some(value) = values.get_mut(segment) else {
                    return Ok(());
                };
                reset_at(ctx, field.ty, value, segments, depth + 1, span)
            }
        }
        TypeNode::Any => reset_dynamic_at(ctx, target, segments, depth, span),
        _ => ctx.fail(span, "evaluation", "cannot traverse primitive target"),
    }
}

#[derive(Clone, Copy)]
enum Policy {
    Hybrid,
    Merge,
}

fn populate(
    ctx: &Context<'_>,
    ty: TypeId,
    target: &mut Value,
    incoming: Value,
    policy: Policy,
    span: Span,
) -> Result<(), ErrorReport> {
    let resolved = ctx
        .schema
        .resolved(ty)
        .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
    let incoming = coerce_dotted(ctx, ty, incoming, span)?;
    match ctx.schema.types[resolved].clone() {
        TypeNode::Any => {
            *target = incoming;
            Ok(())
        }
        TypeNode::String
        | TypeNode::Int
        | TypeNode::Float
        | TypeNode::Bool
        | TypeNode::Literal(_) => {
            ctx.schema
                .validate_value(ty, &incoming, false)
                .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
            *target = incoming;
            Ok(())
        }
        TypeNode::Object(fields) => populate_object(ctx, &fields, target, incoming, policy, span),
        TypeNode::Map(inner) => {
            let Value::Object(source) = incoming else {
                return ctx.fail(span, "evaluation", "expected map value");
            };
            let Value::Object(destination) = target else {
                return ctx.fail(span, "evaluation", "expected map target");
            };
            for (key, value) in source {
                if let Some(existing) = destination.get_mut(&key) {
                    populate(ctx, inner, existing, value, policy, span)?;
                } else {
                    destination.insert(key, materialize(ctx, inner, value, span)?);
                }
            }
            Ok(())
        }
        TypeNode::List { item, key } => {
            let Value::List(source) = incoming else {
                return ctx.fail(span, "evaluation", "expected list value");
            };
            let mut normalized = Vec::new();
            for value in source {
                if let Some(key) = &key {
                    let Value::Object(object) = &value else {
                        return ctx.fail(span, "evaluation", "keyed-list item must be an object");
                    };
                    if !object.contains_key(key) {
                        return ctx.fail(
                            span,
                            "evaluation",
                            format!("keyed-list item must explicitly contain `{key}`"),
                        );
                    }
                }
                normalized.push(materialize(ctx, item, value, span)?);
            }
            ctx.schema
                .validate_value(ty, &Value::List(normalized.clone()), false)
                .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
            let Value::List(destination) = target else {
                return ctx.fail(span, "evaluation", "expected list target");
            };
            if matches!(policy, Policy::Hybrid) {
                *destination = normalized;
            } else if let Some(key) = key {
                for value in normalized {
                    let identity = match &value {
                        Value::Object(object) => {
                            identity_key(object.get(&key).expect("validated identity"))
                                .expect("validated identity type")
                        }
                        _ => unreachable!(),
                    };
                    if let Some(existing) = destination.iter_mut().find(|existing| match existing {
                        Value::Object(object) => {
                            object
                                .get(&key)
                                .and_then(|value| identity_key(value).ok())
                                .as_deref()
                                == Some(&identity)
                        }
                        _ => false,
                    }) {
                        populate(ctx, item, existing, value, Policy::Merge, span)?;
                    } else {
                        destination.push(value);
                    }
                }
            } else {
                destination.extend(normalized);
            }
            Ok(())
        }
        TypeNode::Tuple(items) => {
            let Value::Tuple(source) = incoming else {
                return ctx.fail(span, "evaluation", "expected tuple value");
            };
            if source.len() != items.len() {
                return ctx.fail(
                    span,
                    "evaluation",
                    format!("tuple requires {} elements", items.len()),
                );
            }
            let Value::Tuple(destination) = target else {
                return ctx.fail(span, "evaluation", "expected tuple target");
            };
            for ((ty, destination), value) in items.iter().zip(destination).zip(source) {
                populate(ctx, *ty, destination, value, policy, span)?;
            }
            Ok(())
        }
        TypeNode::Union(branches) => {
            let branch = select_supply(ctx, &branches, &incoming, span)?;
            let old = select_existing(ctx.schema, &branches, target);
            if old != Some(branch) {
                *target = ctx
                    .schema
                    .canonical(branch)
                    .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
            }
            populate(ctx, branch, target, incoming, policy, span)
        }
        TypeNode::Tagged { tag, variants } => {
            let Value::Object(source) = &incoming else {
                return ctx.fail(span, "evaluation", "expected tagged object");
            };
            let Some(Value::String(name)) = source.get(&tag) else {
                return ctx.fail(
                    span,
                    "evaluation",
                    format!("tagged value must explicitly contain `{tag}`"),
                );
            };
            let normalized = name.to_ascii_lowercase();
            let variant = variants
                .iter()
                .find(|variant| variant.name == normalized)
                .ok_or_else(|| {
                    ctx.fail::<()>(
                        span,
                        "evaluation",
                        format!("unknown tagged variant `{name}`"),
                    )
                    .unwrap_err()
                })?;
            let existing = match target {
                Value::Object(object) => match object.get(&tag) {
                    Some(Value::String(value)) => Some(value.as_str()),
                    _ => None,
                },
                _ => None,
            };
            if existing != Some(variant.name.as_str()) {
                *target = tagged_canonical(ctx, &tag, variant, span)?;
            }
            let Value::Object(mut source) = incoming else {
                unreachable!()
            };
            source.insert(tag.clone(), Value::String(variant.name.clone()));
            populate_tagged(ctx, &tag, variant, target, source, policy, span)
        }
        TypeNode::Pending | TypeNode::Ref(_) => unreachable!(),
    }
}

fn coerce_dotted(
    ctx: &Context<'_>,
    ty: TypeId,
    incoming: Value,
    span: Span,
) -> Result<Value, ErrorReport> {
    match (ctx.schema.node(ty)?, incoming) {
        (TypeNode::Tuple(items), Value::Object(entries)) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(
                    ctx.schema.canonical(*item).map_err(|message| {
                        ctx.fail::<()>(span, "evaluation", message).unwrap_err()
                    })?,
                );
            }
            for (key, value) in entries {
                let index = parse_index(ctx, &key, span)?;
                let Some(item) = items.get(index) else {
                    return ctx.fail(span, "evaluation", "tuple index is out of range");
                };
                values[index] = materialize(ctx, *item, value, span)?;
            }
            Ok(Value::Tuple(values))
        }
        (TypeNode::Object(fields), Value::Object(entries)) => {
            let mut result = IndexMap::new();
            for (key, value) in entries {
                let value = if let Some(field) = fields.iter().find(|field| field.name == key) {
                    coerce_dotted(ctx, field.ty, value, span)?
                } else {
                    value
                };
                result.insert(key, value);
            }
            Ok(Value::Object(result))
        }
        (TypeNode::Map(inner), Value::Object(entries)) => {
            let mut result = IndexMap::new();
            for (key, value) in entries {
                result.insert(key, coerce_dotted(ctx, *inner, value, span)?);
            }
            Ok(Value::Object(result))
        }
        (TypeNode::List { .. }, Value::Object(_)) => ctx.fail(
            span,
            "evaluation",
            "dotted shorthand must not traverse a list",
        ),
        (_, value) => Ok(value),
    }
}

fn populate_object(
    ctx: &Context<'_>,
    fields: &[Field],
    target: &mut Value,
    incoming: Value,
    policy: Policy,
    span: Span,
) -> Result<(), ErrorReport> {
    let Value::Object(source) = incoming else {
        return ctx.fail(span, "evaluation", "expected object value");
    };
    let Value::Object(destination) = target else {
        return ctx.fail(span, "evaluation", "expected object target");
    };
    for (key, value) in source {
        let field = fields
            .iter()
            .find(|field| field.name == key)
            .ok_or_else(|| {
                ctx.fail::<()>(
                    span,
                    "evaluation",
                    format!("unknown fixed-object field `{key}`"),
                )
                .unwrap_err()
            })?;
        if let Some(existing) = destination.get_mut(&key) {
            populate(ctx, field.ty, existing, value, policy, span)?;
        } else {
            destination.insert(key, materialize(ctx, field.ty, value, span)?);
        }
    }
    Ok(())
}
fn populate_tagged(
    ctx: &Context<'_>,
    tag: &str,
    variant: &crate::schema::Variant,
    target: &mut Value,
    source: IndexMap<String, Value>,
    policy: Policy,
    span: Span,
) -> Result<(), ErrorReport> {
    let Value::Object(destination) = target else {
        unreachable!()
    };
    for (key, value) in source {
        if key == tag {
            destination.insert(key, Value::String(variant.name.clone()));
            continue;
        }
        let field = variant
            .fields
            .iter()
            .find(|field| field.name == key)
            .ok_or_else(|| {
                ctx.fail::<()>(span, "evaluation", format!("unknown tagged field `{key}`"))
                    .unwrap_err()
            })?;
        if let Some(existing) = destination.get_mut(&key) {
            populate(ctx, field.ty, existing, value, policy, span)?;
        } else {
            destination.insert(key, materialize(ctx, field.ty, value, span)?);
        }
    }
    Ok(())
}
fn materialize(
    ctx: &Context<'_>,
    ty: TypeId,
    incoming: Value,
    span: Span,
) -> Result<Value, ErrorReport> {
    let mut target = fresh_for(ctx, ty, &incoming, span)?;
    populate(ctx, ty, &mut target, incoming, Policy::Hybrid, span)?;
    ctx.schema
        .validate_complete(ty, &target)
        .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?;
    Ok(target)
}
fn fresh_for(
    ctx: &Context<'_>,
    ty: TypeId,
    incoming: &Value,
    span: Span,
) -> Result<Value, ErrorReport> {
    match ctx.schema.node(ty)? {
        TypeNode::Any => Ok(incoming.clone()),
        TypeNode::Union(branches) => {
            let branch = select_supply(ctx, branches, incoming, span)?;
            ctx.schema
                .canonical(branch)
                .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())
        }
        TypeNode::Tagged { tag, variants } => {
            let Value::Object(object) = incoming else {
                return ctx.fail(span, "evaluation", "expected tagged object");
            };
            let Some(Value::String(name)) = object.get(tag) else {
                return ctx.fail(
                    span,
                    "evaluation",
                    format!("tagged value must explicitly contain `{tag}`"),
                );
            };
            let variant = variants
                .iter()
                .find(|variant| variant.name == name.to_ascii_lowercase())
                .ok_or_else(|| {
                    ctx.fail::<()>(span, "evaluation", "unknown tagged variant")
                        .unwrap_err()
                })?;
            tagged_canonical(ctx, tag, variant, span)
        }
        _ => ctx
            .schema
            .canonical(ty)
            .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err()),
    }
}
fn tagged_canonical(
    ctx: &Context<'_>,
    tag: &str,
    variant: &crate::schema::Variant,
    span: Span,
) -> Result<Value, ErrorReport> {
    let mut object = IndexMap::new();
    object.insert(tag.to_owned(), Value::String(variant.name.clone()));
    for field in variant.fields.iter().filter(|field| !field.optional) {
        object.insert(
            field.name.clone(),
            ctx.schema
                .canonical(field.ty)
                .map_err(|message| ctx.fail::<()>(span, "evaluation", message).unwrap_err())?,
        );
    }
    Ok(Value::Object(object))
}
fn select_supply(
    ctx: &Context<'_>,
    branches: &[TypeId],
    incoming: &Value,
    span: Span,
) -> Result<TypeId, ErrorReport> {
    let matches: Vec<_> = branches
        .iter()
        .copied()
        .filter(|branch| {
            let mut candidate = match ctx.schema.canonical(*branch) {
                Ok(value) => value,
                Err(_) => return false,
            };
            populate(
                ctx,
                *branch,
                &mut candidate,
                incoming.clone(),
                Policy::Hybrid,
                span,
            )
            .is_ok()
                && ctx.schema.validate_complete(*branch, &candidate).is_ok()
        })
        .collect();
    match matches.as_slice() {
        [branch] => Ok(*branch),
        [] => ctx.fail(span, "evaluation", "value matches no union branch"),
        _ => ctx.fail(
            span,
            "evaluation",
            "value ambiguously matches multiple union branches",
        ),
    }
}
fn select_existing(schema: &Schema, branches: &[TypeId], value: &Value) -> Option<TypeId> {
    branches
        .iter()
        .copied()
        .find(|branch| schema.validate_complete(*branch, value).is_ok())
}

fn apply_dynamic_at(
    ctx: &Context<'_>,
    target: &mut Value,
    segments: &[String],
    depth: usize,
    incoming: Value,
    span: Span,
) -> Result<(), ErrorReport> {
    if depth == segments.len() {
        *target = incoming;
        return Ok(());
    }
    let segment = &segments[depth];
    match target {
        Value::Object(values) => {
            if !values.contains_key(segment) {
                values.insert(segment.clone(), incoming.clone());
            }
            apply_dynamic_at(
                ctx,
                values.get_mut(segment).expect("inserted"),
                segments,
                depth + 1,
                incoming,
                span,
            )
        }
        Value::List(values) | Value::Tuple(values) => {
            let index = parse_index(ctx, segment, span)?;
            let value = values.get_mut(index).ok_or_else(|| {
                ctx.fail::<()>(span, "evaluation", "index is out of range")
                    .unwrap_err()
            })?;
            apply_dynamic_at(ctx, value, segments, depth + 1, incoming, span)
        }
        _ => ctx.fail(span, "evaluation", "cannot traverse primitive `any` value"),
    }
}
fn reset_dynamic_at(
    ctx: &Context<'_>,
    target: &mut Value,
    segments: &[String],
    depth: usize,
    span: Span,
) -> Result<(), ErrorReport> {
    let segment = &segments[depth];
    match target {
        Value::Object(values) => {
            if depth + 1 == segments.len() {
                values.shift_remove(segment);
                Ok(())
            } else {
                let Some(value) = values.get_mut(segment) else {
                    return Ok(());
                };
                reset_dynamic_at(ctx, value, segments, depth + 1, span)
            }
        }
        Value::List(values) => {
            let index = parse_index(ctx, segment, span)?;
            if index >= values.len() {
                return ctx.fail(span, "evaluation", "index is out of range");
            }
            if depth + 1 == segments.len() {
                values.remove(index);
                Ok(())
            } else {
                reset_dynamic_at(ctx, &mut values[index], segments, depth + 1, span)
            }
        }
        Value::Tuple(_) => ctx.fail(
            span,
            "evaluation",
            "cannot canonically reset an element of a tuple stored in `any`",
        ),
        _ => ctx.fail(span, "evaluation", "cannot traverse primitive `any` value"),
    }
}
fn parse_index(ctx: &Context<'_>, value: &str, span: Span) -> Result<usize, ErrorReport> {
    value.parse().map_err(|_| {
        ctx.fail::<()>(span, "evaluation", "expected integer index")
            .unwrap_err()
    })
}

impl Value {
    fn static_type(&self) -> StaticType {
        match self {
            Value::String(_) => StaticType::String,
            Value::Int(_) => StaticType::Int,
            Value::Float(_) => StaticType::Float,
            Value::Bool(_) => StaticType::Bool,
            Value::Object(_) => StaticType::Object,
            Value::List(_) => StaticType::List,
            Value::Tuple(_) => StaticType::Tuple,
        }
    }
}
