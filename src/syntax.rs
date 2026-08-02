// SPDX-License-Identifier: MPL-2.0

use crate::diagnostic::{ErrorReport, Span, diagnostic};
use crate::{Source, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Node<T> {
    pub value: T,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) struct SchemaAst {
    pub declarations: Vec<Node<Declaration>>,
}

#[derive(Clone, Debug)]
pub(crate) enum Declaration {
    Type(String, TyExpr),
    Schema(String, TyExpr),
}

#[derive(Clone, Debug)]
pub(crate) struct FieldExpr {
    pub name: String,
    pub spelling: String,
    pub optional: bool,
    pub ty: TyExpr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum ObjectShape {
    Inline(Vec<FieldExpr>),
    Name(String, Span),
}

#[derive(Clone, Debug)]
pub(crate) enum TyExpr {
    String(Span),
    Int(Span),
    Float(Span),
    Bool(Span),
    Any(Span),
    Literal(Value, Span),
    Name(String, Span),
    Object(Vec<FieldExpr>, Span),
    Map(Box<TyExpr>, Span),
    List(Box<TyExpr>, Span),
    Tuple(Vec<TyExpr>, Span),
    Union(Vec<TyExpr>, Span),
    Keyed(Box<TyExpr>, String, Span),
    Tagged {
        tag: String,
        common: Option<ObjectShape>,
        variants: Vec<(String, String, ObjectShape, Span)>,
        span: Span,
    },
}

impl TyExpr {
    pub fn span(&self) -> Span {
        match self {
            Self::String(s)
            | Self::Int(s)
            | Self::Float(s)
            | Self::Bool(s)
            | Self::Any(s)
            | Self::Literal(_, s)
            | Self::Name(_, s)
            | Self::Object(_, s)
            | Self::Map(_, s)
            | Self::List(_, s)
            | Self::Tuple(_, s)
            | Self::Union(_, s)
            | Self::Keyed(_, _, s) => *s,
            Self::Tagged { span, .. } => *span,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OverlayAst {
    pub locator: Node<String>,
    pub overlays: Vec<Node<Overlay>>,
}

#[derive(Clone, Debug)]
pub(crate) struct Overlay {
    pub spelling: String,
    pub statements: Vec<Node<Statement>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Path {
    pub segments: Vec<PathSegment>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PathSegment {
    pub value: String,
    pub quoted: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ActionKind {
    Assign,
    Merge,
    Set,
    Reset,
}

#[derive(Clone, Debug)]
pub(crate) enum Statement {
    Action(ActionKind, Path, Option<Expr>),
    If(
        Vec<(Expr, Vec<Node<Statement>>)>,
        Option<Vec<Node<Statement>>>,
    ),
    For(String, String, Expr, Vec<Node<Statement>>),
}

#[derive(Clone, Debug)]
pub(crate) struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub(crate) enum ExprKind {
    Literal(Value),
    Path(Path),
    Variable(String),
    Object(Vec<(Path, Expr)>),
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum UnaryOp {
    Not,
    Negate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Or,
    And,
    Equal,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Clone, Debug, PartialEq)]
enum TokenKind {
    Word(String),
    Int(String),
    Float(String),
    String(String),
    Symbol(&'static str),
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    span: Span,
}

fn norm(value: &str) -> String {
    value.to_ascii_lowercase()
}

fn lex(source: &Source) -> Result<Vec<Token>, ErrorReport> {
    let text = source.text.as_str();
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let start = pos;
        match bytes[pos] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                pos += 1;
            }
            b'#' => {
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
            }
            b'"' if text[start..].starts_with("\"\"\"") => {
                let body_start = start + 3;
                pos = body_start;
                let mut body_end = None;
                while pos < bytes.len() {
                    if text[pos..].starts_with("\"\"\"") {
                        body_end = Some(pos);
                        pos += 3;
                        break;
                    }
                    let ch = text[pos..].chars().next().expect("character boundary");
                    pos += ch.len_utf8();
                    if ch == '\\' && pos < bytes.len() {
                        let escaped = text[pos..].chars().next().expect("character boundary");
                        pos += escaped.len_utf8();
                    }
                }
                let Some(body_end) = body_end else {
                    return Err(ErrorReport::one(diagnostic(
                        source,
                        Span { start, end: pos },
                        "syntax",
                        "unterminated multiline string",
                    )));
                };
                let body = dedent_multiline(&text[body_start..body_end]);
                let value = decode_multiline_string(source, &body, Span { start, end: pos })?;
                tokens.push(Token {
                    kind: TokenKind::String(value),
                    span: Span { start, end: pos },
                });
            }
            b'"' => {
                pos += 1;
                let mut value = String::new();
                let mut closed = false;
                while pos < bytes.len() {
                    if bytes[pos] == b'"' {
                        pos += 1;
                        closed = true;
                        break;
                    }
                    if bytes[pos] == b'\n' || bytes[pos] == b'\r' {
                        break;
                    }
                    if bytes[pos] != b'\\' {
                        let ch = text[pos..].chars().next().expect("character boundary");
                        value.push(ch);
                        pos += ch.len_utf8();
                        continue;
                    }
                    pos += 1;
                    if pos >= bytes.len() {
                        break;
                    }
                    match bytes[pos] {
                        b'"' => value.push('"'),
                        b'\\' => value.push('\\'),
                        b'n' => value.push('\n'),
                        b'r' => value.push('\r'),
                        b't' => value.push('\t'),
                        b'u' => {
                            let (unit, end) = unicode_unit(source, pos + 1, start)?;
                            pos = end - 1;
                            if (0xd800..=0xdbff).contains(&unit) {
                                if bytes.get(pos + 1..pos + 3) != Some(b"\\u") {
                                    return Err(ErrorReport::one(diagnostic(
                                        source,
                                        Span {
                                            start,
                                            end: pos + 1,
                                        },
                                        "syntax",
                                        "high surrogate must be followed by a low surrogate",
                                    )));
                                }
                                let (low, low_end) = unicode_unit(source, pos + 3, start)?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err(ErrorReport::one(diagnostic(
                                        source,
                                        Span {
                                            start,
                                            end: low_end,
                                        },
                                        "syntax",
                                        "invalid Unicode surrogate pair",
                                    )));
                                }
                                let scalar = 0x10000
                                    + (((unit as u32 - 0xd800) << 10) | (low as u32 - 0xdc00));
                                value.push(char::from_u32(scalar).expect("valid scalar"));
                                pos = low_end - 1;
                            } else if (0xdc00..=0xdfff).contains(&unit) {
                                return Err(ErrorReport::one(diagnostic(
                                    source,
                                    Span { start, end },
                                    "syntax",
                                    "unexpected low surrogate",
                                )));
                            } else {
                                value.push(char::from_u32(unit as u32).expect("valid scalar"));
                            }
                        }
                        _ => {
                            return Err(ErrorReport::one(diagnostic(
                                source,
                                Span {
                                    start,
                                    end: pos + 1,
                                },
                                "syntax",
                                "unknown string escape",
                            )));
                        }
                    }
                    pos += 1;
                }
                if !closed {
                    return Err(ErrorReport::one(diagnostic(
                        source,
                        Span { start, end: pos },
                        "syntax",
                        "unterminated string",
                    )));
                }
                tokens.push(Token {
                    kind: TokenKind::String(value),
                    span: Span { start, end: pos },
                });
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
                pos += 1;
                loop {
                    while pos < bytes.len()
                        && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_')
                    {
                        pos += 1;
                    }
                    if pos + 1 < bytes.len()
                        && bytes[pos] == b'-'
                        && (bytes[pos + 1].is_ascii_alphanumeric() || bytes[pos + 1] == b'_')
                    {
                        pos += 1;
                    } else {
                        break;
                    }
                }
                let raw = &text[start..pos];
                let all_digits = raw.bytes().all(|byte| byte.is_ascii_digit());
                if all_digits
                    && bytes.get(pos) == Some(&b'.')
                    && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit)
                {
                    pos += 1;
                    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                        pos += 1;
                    }
                    tokens.push(Token {
                        kind: TokenKind::Float(text[start..pos].to_owned()),
                        span: Span { start, end: pos },
                    });
                } else if all_digits {
                    tokens.push(Token {
                        kind: TokenKind::Int(raw.to_owned()),
                        span: Span { start, end: pos },
                    });
                } else {
                    tokens.push(Token {
                        kind: TokenKind::Word(raw.to_owned()),
                        span: Span { start, end: pos },
                    });
                }
            }
            _ => {
                let (symbol, width) = if text[start..].starts_with("<=") {
                    ("<=", 2)
                } else if text[start..].starts_with(">=") {
                    (">=", 2)
                } else if text[start..].starts_with("==") {
                    ("==", 2)
                } else {
                    let symbol = match bytes[pos] {
                        b'=' => "=",
                        b'<' => "<",
                        b'>' => ">",
                        b'+' => "+",
                        b'-' => "-",
                        b'*' => "*",
                        b'/' => "/",
                        b'|' => "|",
                        b'?' => "?",
                        b'.' => ".",
                        b';' => ";",
                        b'{' => "{",
                        b'}' => "}",
                        b'[' => "[",
                        b']' => "]",
                        b'(' => "(",
                        b')' => ")",
                        _ => {
                            return Err(ErrorReport::one(diagnostic(
                                source,
                                Span {
                                    start,
                                    end: start + 1,
                                },
                                "syntax",
                                "unexpected character",
                            )));
                        }
                    };
                    (symbol, 1)
                };
                pos += width;
                tokens.push(Token {
                    kind: TokenKind::Symbol(symbol),
                    span: Span { start, end: pos },
                });
            }
        }
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        span: Span {
            start: pos,
            end: pos,
        },
    });
    Ok(tokens)
}

fn unicode_unit(
    source: &Source,
    start: usize,
    string_start: usize,
) -> Result<(u16, usize), ErrorReport> {
    let end = start + 4;
    let Some(raw) = source.text.get(start..end) else {
        return Err(ErrorReport::one(diagnostic(
            source,
            Span {
                start: string_start,
                end: source.text.len(),
            },
            "syntax",
            "incomplete Unicode escape",
        )));
    };
    let value = u16::from_str_radix(raw, 16).map_err(|_| {
        ErrorReport::one(diagnostic(
            source,
            Span { start, end },
            "syntax",
            "Unicode escape requires four hexadecimal digits",
        ))
    })?;
    Ok((value, end))
}

fn dedent_multiline(raw: &str) -> String {
    let raw = if let Some(rest) = raw.strip_prefix("\r\n") {
        rest
    } else if let Some(rest) = raw.strip_prefix(['\r', '\n']) {
        rest
    } else {
        raw
    };

    let lines = physical_lines(raw);
    let mut common: Option<&str> = None;
    for (content, _) in &lines {
        if content.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
            continue;
        }
        let indent_len = content
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let indent = &content[..indent_len];
        common = Some(match common {
            None => indent,
            Some(current) => {
                &current[..current
                    .bytes()
                    .zip(indent.bytes())
                    .take_while(|(left, right)| left == right)
                    .count()]
            }
        });
    }
    let common = common.unwrap_or_default();

    let mut result = String::with_capacity(raw.len());
    for (content, ending) in lines {
        if content.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
            result.push_str(ending);
        } else {
            result.push_str(&content[common.len()..]);
            result.push_str(ending);
        }
    }
    result
}

fn physical_lines(mut raw: &str) -> Vec<(&str, &str)> {
    let mut lines = Vec::new();
    while !raw.is_empty() {
        let end = raw
            .bytes()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .unwrap_or(raw.len());
        let (content, rest) = raw.split_at(end);
        let ending_len = if rest.starts_with("\r\n") {
            2
        } else if rest.is_empty() {
            0
        } else {
            1
        };
        let (ending, remaining) = rest.split_at(ending_len);
        lines.push((content, ending));
        raw = remaining;
    }
    lines
}

fn decode_multiline_string(source: &Source, raw: &str, span: Span) -> Result<String, ErrorReport> {
    let bytes = raw.as_bytes();
    let mut value = String::new();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] != b'\\' {
            let ch = raw[pos..].chars().next().expect("character boundary");
            value.push(ch);
            pos += ch.len_utf8();
            continue;
        }

        pos += 1;
        let Some(escape) = bytes.get(pos).copied() else {
            return Err(multiline_string_error(
                source,
                span,
                "incomplete string escape",
            ));
        };
        match escape {
            b'"' => {
                value.push('"');
                pos += 1;
            }
            b'\\' => {
                value.push('\\');
                pos += 1;
            }
            b'n' => {
                value.push('\n');
                pos += 1;
            }
            b'r' => {
                value.push('\r');
                pos += 1;
            }
            b't' => {
                value.push('\t');
                pos += 1;
            }
            b'u' => {
                let (unit, end) = multiline_unicode_unit(source, raw, pos + 1, span)?;
                pos = end;
                if (0xd800..=0xdbff).contains(&unit) {
                    if raw.as_bytes().get(pos..pos + 2) != Some(b"\\u") {
                        return Err(multiline_string_error(
                            source,
                            span,
                            "high surrogate must be followed by a low surrogate",
                        ));
                    }
                    let (low, low_end) = multiline_unicode_unit(source, raw, pos + 2, span)?;
                    if !(0xdc00..=0xdfff).contains(&low) {
                        return Err(multiline_string_error(
                            source,
                            span,
                            "invalid Unicode surrogate pair",
                        ));
                    }
                    let scalar = 0x10000 + (((unit as u32 - 0xd800) << 10) | (low as u32 - 0xdc00));
                    value.push(char::from_u32(scalar).expect("valid scalar"));
                    pos = low_end;
                } else if (0xdc00..=0xdfff).contains(&unit) {
                    return Err(multiline_string_error(
                        source,
                        span,
                        "unexpected low surrogate",
                    ));
                } else {
                    value.push(char::from_u32(unit as u32).expect("valid scalar"));
                }
            }
            _ => {
                return Err(multiline_string_error(
                    source,
                    span,
                    "unknown string escape",
                ));
            }
        }
    }
    Ok(value)
}

fn multiline_unicode_unit(
    source: &Source,
    raw: &str,
    start: usize,
    span: Span,
) -> Result<(u16, usize), ErrorReport> {
    let end = start + 4;
    let Some(digits) = raw.get(start..end) else {
        return Err(multiline_string_error(
            source,
            span,
            "incomplete Unicode escape",
        ));
    };
    let value = u16::from_str_radix(digits, 16).map_err(|_| {
        multiline_string_error(
            source,
            span,
            "Unicode escape requires four hexadecimal digits",
        )
    })?;
    Ok((value, end))
}

fn multiline_string_error(source: &Source, span: Span, message: &str) -> ErrorReport {
    ErrorReport::one(diagnostic(source, span, "syntax", message))
}

struct Parser<'a> {
    source: &'a Source,
    tokens: Vec<Token>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a Source) -> Result<Self, ErrorReport> {
        Ok(Self {
            source,
            tokens: lex(source)?,
            pos: 0,
        })
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }
    fn bump(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        self.pos += 1;
        token
    }
    fn error<T>(&self, message: impl Into<String>) -> Result<T, ErrorReport> {
        Err(ErrorReport::one(diagnostic(
            self.source,
            self.current().span,
            "syntax",
            message,
        )))
    }
    fn is_symbol(&self, value: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Symbol(v) if *v == value)
    }
    fn eat_symbol(&mut self, value: &str) -> bool {
        if self.is_symbol(value) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn symbol(&mut self, value: &str) -> Result<Token, ErrorReport> {
        if self.is_symbol(value) {
            Ok(self.bump())
        } else {
            self.error(format!("expected `{value}`"))
        }
    }
    fn is_word(&self, value: &str) -> bool {
        matches!(&self.current().kind, TokenKind::Word(v) if v == value)
    }
    fn eat_word(&mut self, value: &str) -> bool {
        if self.is_word(value) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn word(&mut self, value: &str) -> Result<Token, ErrorReport> {
        if self.is_word(value) {
            Ok(self.bump())
        } else {
            self.error(format!("expected `{value}`"))
        }
    }
    fn name(&mut self) -> Result<(String, String, Span), ErrorReport> {
        match self.bump() {
            Token {
                kind: TokenKind::Word(spelling),
                span,
            } => Ok((norm(&spelling), spelling, span)),
            token => {
                self.pos -= 1;
                self.error(format!("expected name, found {}", token_label(&token.kind)))
            }
        }
    }
    fn string(&mut self) -> Result<(String, Span), ErrorReport> {
        match self.bump() {
            Token {
                kind: TokenKind::String(value),
                span,
            } => Ok((value, span)),
            _ => {
                self.pos -= 1;
                self.error("expected string")
            }
        }
    }

    fn schema(mut self) -> Result<SchemaAst, ErrorReport> {
        let mut declarations = Vec::new();
        while !matches!(self.current().kind, TokenKind::Eof) {
            let start = self.current().span.start;
            let declaration = if self.eat_word("type") {
                let (name, _, _) = self.name()?;
                self.symbol("=")?;
                let ty = self.ty()?;
                let end = self.symbol(";")?.span.end;
                Node {
                    value: Declaration::Type(name, ty),
                    span: Span { start, end },
                }
            } else if self.eat_word("schema") {
                let (name, _, _) = self.name()?;
                self.symbol("=")?;
                let ty = self.ty()?;
                let end = self.symbol(";")?.span.end;
                Node {
                    value: Declaration::Schema(name, ty),
                    span: Span { start, end },
                }
            } else {
                return self.error("expected `type` or `schema` declaration");
            };
            declarations.push(declaration);
        }
        Ok(SchemaAst { declarations })
    }

    fn overlay(mut self) -> Result<OverlayAst, ErrorReport> {
        self.word("schema")?;
        self.symbol("=")?;
        let (locator, span) = self.string()?;
        self.symbol(";")?;
        let mut overlays = Vec::new();
        while !matches!(self.current().kind, TokenKind::Eof) {
            let start = self.word("overlay")?.span.start;
            let (_, spelling, _) = self.name()?;
            self.symbol("=")?;
            let statements = self.block()?;
            let end = self.symbol(";")?.span.end;
            overlays.push(Node {
                value: Overlay {
                    spelling,
                    statements,
                },
                span: Span { start, end },
            });
        }
        Ok(OverlayAst {
            locator: Node {
                value: locator,
                span,
            },
            overlays,
        })
    }

    fn ty(&mut self) -> Result<TyExpr, ErrorReport> {
        let first = self.keyed_ty()?;
        if !self.eat_symbol("|") {
            return Ok(first);
        }
        let start = first.span().start;
        let mut values = vec![first];
        loop {
            values.push(self.keyed_ty()?);
            if !self.eat_symbol("|") {
                break;
            }
        }
        let end = values.last().expect("union branch").span().end;
        Ok(TyExpr::Union(values, Span { start, end }))
    }

    fn keyed_ty(&mut self) -> Result<TyExpr, ErrorReport> {
        let ty = self.primary_ty()?;
        if self.eat_word("key") {
            let (name, _, span) = self.name()?;
            let start = ty.span().start;
            Ok(TyExpr::Keyed(
                Box::new(ty),
                name,
                Span {
                    start,
                    end: span.end,
                },
            ))
        } else {
            Ok(ty)
        }
    }

    fn primary_ty(&mut self) -> Result<TyExpr, ErrorReport> {
        let start = self.current().span.start;
        for (word, make) in [
            ("string", TyExpr::String as fn(Span) -> TyExpr),
            ("int", TyExpr::Int),
            ("float", TyExpr::Float),
            ("bool", TyExpr::Bool),
            ("any", TyExpr::Any),
        ] {
            if self.eat_word(word) {
                return Ok(make(self.tokens[self.pos - 1].span));
            }
        }
        if self.eat_word("list") {
            self.symbol("<")?;
            let inner = self.ty()?;
            let end = self.symbol(">")?.span.end;
            return Ok(TyExpr::List(Box::new(inner), Span { start, end }));
        }
        if self.eat_word("map") {
            self.symbol("<")?;
            let inner = self.ty()?;
            let end = self.symbol(">")?.span.end;
            return Ok(TyExpr::Map(Box::new(inner), Span { start, end }));
        }
        if self.eat_word("tuple") {
            self.symbol("<")?;
            let mut items = Vec::new();
            while !self.eat_symbol(">") {
                items.push(self.ty()?);
                self.symbol(";")?;
            }
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(TyExpr::Tuple(items, Span { start, end }));
        }
        if self.eat_word("tagged") {
            return self.tagged_ty(start);
        }
        if self.is_symbol("{") {
            let fields = self.object_fields()?;
            let end = self.tokens[self.pos - 1].span.end;
            return Ok(TyExpr::Object(fields, Span { start, end }));
        }
        if self.eat_symbol("(") {
            let ty = self.ty()?;
            let end = self.symbol(")")?.span.end;
            return Ok(with_span(ty, Span { start, end }));
        }
        match self.current().kind.clone() {
            TokenKind::String(value) => {
                let span = self.bump().span;
                Ok(TyExpr::Literal(Value::String(value), span))
            }
            TokenKind::Int(_) | TokenKind::Float(_) | TokenKind::Symbol("-") => {
                let (value, span) = self.number_literal()?;
                Ok(TyExpr::Literal(value, span))
            }
            TokenKind::Word(ref value) if value == "true" || value == "false" => {
                let token = self.bump();
                Ok(TyExpr::Literal(Value::Bool(value == "true"), token.span))
            }
            TokenKind::Word(_) => {
                let (name, _, span) = self.name()?;
                Ok(TyExpr::Name(name, span))
            }
            _ => self.error("expected type expression"),
        }
    }

    fn tagged_ty(&mut self, start: usize) -> Result<TyExpr, ErrorReport> {
        self.symbol("{")?;
        self.word("tag")?;
        self.symbol("=")?;
        let (tag, _, _) = self.name()?;
        self.symbol(";")?;
        let common = if self.eat_word("common") {
            self.symbol("=")?;
            let v = self.object_shape()?;
            self.symbol(";")?;
            Some(v)
        } else {
            None
        };
        self.word("variants")?;
        self.symbol("=")?;
        self.symbol("{")?;
        let mut variants = Vec::new();
        while !self.eat_symbol("}") {
            let (name, spelling, span) = self.name()?;
            self.symbol("=")?;
            let shape = self.object_shape()?;
            let end = self.symbol(";")?.span.end;
            variants.push((
                name,
                spelling,
                shape,
                Span {
                    start: span.start,
                    end,
                },
            ));
        }
        if variants.is_empty() {
            return self.error("tagged union requires at least one variant");
        }
        self.symbol(";")?;
        let end = self.symbol("}")?.span.end;
        Ok(TyExpr::Tagged {
            tag,
            common,
            variants,
            span: Span { start, end },
        })
    }

    fn object_shape(&mut self) -> Result<ObjectShape, ErrorReport> {
        if self.is_symbol("{") {
            Ok(ObjectShape::Inline(self.object_fields()?))
        } else {
            let (name, _, span) = self.name()?;
            Ok(ObjectShape::Name(name, span))
        }
    }

    fn object_fields(&mut self) -> Result<Vec<FieldExpr>, ErrorReport> {
        self.symbol("{")?;
        let mut fields = Vec::new();
        while !self.eat_symbol("}") {
            let (name, spelling, span) = self.name()?;
            let optional = self.eat_symbol("?");
            self.symbol("=")?;
            let ty = self.ty()?;
            let end = self.symbol(";")?.span.end;
            fields.push(FieldExpr {
                name,
                spelling,
                optional,
                ty,
                span: Span {
                    start: span.start,
                    end,
                },
            });
        }
        Ok(fields)
    }

    fn block(&mut self) -> Result<Vec<Node<Statement>>, ErrorReport> {
        self.symbol("{")?;
        let mut statements = Vec::new();
        while !self.eat_symbol("}") {
            statements.push(self.statement()?);
        }
        Ok(statements)
    }

    fn statement(&mut self) -> Result<Node<Statement>, ErrorReport> {
        let start = self.current().span.start;
        if self.eat_word("if") {
            let mut branches = Vec::new();
            let condition = self.expr()?;
            let body = self.block()?;
            branches.push((condition, body));
            let mut otherwise = None;
            while self.eat_word("else") {
                if self.eat_word("if") {
                    branches.push((self.expr()?, self.block()?));
                } else {
                    otherwise = Some(self.block()?);
                    break;
                }
            }
            let end = self.symbol(";")?.span.end;
            return Ok(Node {
                value: Statement::If(branches, otherwise),
                span: Span { start, end },
            });
        }
        if self.eat_word("for") {
            let (name, spelling, _) = self.name()?;
            self.word("in")?;
            let iterable = self.expr()?;
            let body = self.block()?;
            let end = self.symbol(";")?.span.end;
            return Ok(Node {
                value: Statement::For(name, spelling, iterable, body),
                span: Span { start, end },
            });
        }
        let kind = if self.eat_word("merge") {
            ActionKind::Merge
        } else if self.eat_word("set") {
            ActionKind::Set
        } else if self.eat_word("reset") {
            ActionKind::Reset
        } else {
            ActionKind::Assign
        };
        let path = self.path()?;
        let expr = if matches!(kind, ActionKind::Reset) {
            None
        } else {
            self.symbol("=")?;
            Some(self.expr()?)
        };
        let end = self.symbol(";")?.span.end;
        Ok(Node {
            value: Statement::Action(kind, path, expr),
            span: Span { start, end },
        })
    }

    fn path(&mut self) -> Result<Path, ErrorReport> {
        let start = self.symbol(".")?.span.start;
        let mut segments = Vec::new();
        if matches!(
            self.current().kind,
            TokenKind::Word(_) | TokenKind::Int(_) | TokenKind::String(_)
        ) {
            loop {
                let token = self.bump();
                let (value, quoted) = match token.kind {
                    TokenKind::Word(v) | TokenKind::Int(v) => (norm(&v), false),
                    TokenKind::String(v) => (v, true),
                    _ => unreachable!(),
                };
                segments.push(PathSegment {
                    value,
                    quoted,
                    span: token.span,
                });
                if !self.eat_symbol(".") {
                    break;
                }
            }
        }
        let end = segments
            .last()
            .map_or(start + 1, |segment| segment.span.end);
        Ok(Path {
            segments,
            span: Span { start, end },
        })
    }

    fn relative_path(&mut self) -> Result<Path, ErrorReport> {
        let token = self.bump();
        let start = token.span.start;
        let (first, quoted) = match token.kind {
            TokenKind::Word(v) => (norm(&v), false),
            TokenKind::String(v) => (v, true),
            _ => {
                self.pos -= 1;
                return self.error("expected relative field name or quoted map key");
            }
        };
        let mut segments = vec![PathSegment {
            value: first,
            quoted,
            span: token.span,
        }];
        while self.eat_symbol(".") {
            let token = self.bump();
            let (value, quoted) = match token.kind {
                TokenKind::Word(v) | TokenKind::Int(v) => (norm(&v), false),
                TokenKind::String(v) => (v, true),
                _ => {
                    self.pos -= 1;
                    return self.error("expected path segment");
                }
            };
            segments.push(PathSegment {
                value,
                quoted,
                span: token.span,
            });
        }
        let end = segments.last().expect("first segment").span.end;
        Ok(Path {
            segments,
            span: Span { start, end },
        })
    }

    fn expr(&mut self) -> Result<Expr, ErrorReport> {
        self.binary(0)
    }
    fn binary(&mut self, min_precedence: u8) -> Result<Expr, ErrorReport> {
        let mut left = self.unary()?;
        while let Some((op, precedence)) = self.binary_op() {
            if precedence < min_precedence {
                break;
            }
            self.bump();
            let right = self.binary(precedence + 1)?;
            let span = Span {
                start: left.span.start,
                end: right.span.end,
            };
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }
    fn binary_op(&self) -> Option<(BinaryOp, u8)> {
        let value = match &self.current().kind {
            TokenKind::Word(v) => v.as_str(),
            TokenKind::Symbol(v) => *v,
            _ => return None,
        };
        Some(match value {
            "or" => (BinaryOp::Or, 1),
            "and" => (BinaryOp::And, 2),
            "==" => (BinaryOp::Equal, 3),
            "<" => (BinaryOp::Less, 4),
            ">" => (BinaryOp::Greater, 4),
            "<=" => (BinaryOp::LessEqual, 4),
            ">=" => (BinaryOp::GreaterEqual, 4),
            "+" => (BinaryOp::Add, 5),
            "-" => (BinaryOp::Subtract, 5),
            "*" => (BinaryOp::Multiply, 6),
            "/" => (BinaryOp::Divide, 6),
            _ => return None,
        })
    }
    fn unary(&mut self) -> Result<Expr, ErrorReport> {
        if self.eat_word("not") {
            let start = self.tokens[self.pos - 1].span.start;
            let value = self.unary()?;
            let span = Span {
                start,
                end: value.span.end,
            };
            return Ok(Expr {
                kind: ExprKind::Unary(UnaryOp::Not, Box::new(value)),
                span,
            });
        }
        if self.eat_symbol("-") {
            let start = self.tokens[self.pos - 1].span.start;
            if let TokenKind::Int(raw) = &self.current().kind
                && raw == "9223372036854775808"
            {
                let end = self.bump().span.end;
                return Ok(Expr {
                    kind: ExprKind::Literal(Value::Int(i64::MIN)),
                    span: Span { start, end },
                });
            }
            let value = self.unary()?;
            let span = Span {
                start,
                end: value.span.end,
            };
            return Ok(Expr {
                kind: ExprKind::Unary(UnaryOp::Negate, Box::new(value)),
                span,
            });
        }
        self.primary_expr()
    }
    fn primary_expr(&mut self) -> Result<Expr, ErrorReport> {
        let start = self.current().span.start;
        match self.current().kind.clone() {
            TokenKind::String(value) => {
                let span = self.bump().span;
                Ok(Expr {
                    kind: ExprKind::Literal(Value::String(value)),
                    span,
                })
            }
            TokenKind::Int(raw) => {
                let span = self.bump().span;
                let value = parse_positive_i64(self.source, &raw, span)?;
                Ok(Expr {
                    kind: ExprKind::Literal(Value::Int(value)),
                    span,
                })
            }
            TokenKind::Float(raw) => {
                let span = self.bump().span;
                let value = raw.parse::<f64>().map_err(|_| {
                    ErrorReport::one(diagnostic(
                        self.source,
                        span,
                        "syntax",
                        "invalid float literal",
                    ))
                })?;
                Ok(Expr {
                    kind: ExprKind::Literal(Value::Float(value)),
                    span,
                })
            }
            TokenKind::Word(ref value) if value == "true" || value == "false" => {
                let value = value == "true";
                let span = self.bump().span;
                Ok(Expr {
                    kind: ExprKind::Literal(Value::Bool(value)),
                    span,
                })
            }
            TokenKind::Word(_) => {
                let (name, _, span) = self.name()?;
                Ok(Expr {
                    kind: ExprKind::Variable(name),
                    span,
                })
            }
            TokenKind::Symbol(".") => {
                let path = self.path()?;
                let span = path.span;
                Ok(Expr {
                    kind: ExprKind::Path(path),
                    span,
                })
            }
            TokenKind::Symbol("{") => {
                self.bump();
                let mut entries = Vec::new();
                while !self.eat_symbol("}") {
                    let path = self.relative_path()?;
                    self.symbol("=")?;
                    let value = self.expr()?;
                    self.symbol(";")?;
                    entries.push((path, value));
                }
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Expr {
                    kind: ExprKind::Object(entries),
                    span: Span { start, end },
                })
            }
            TokenKind::Symbol("[") => {
                self.bump();
                let mut items = Vec::new();
                while !self.eat_symbol("]") {
                    items.push(self.expr()?);
                    self.symbol(";")?;
                }
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Expr {
                    kind: ExprKind::List(items),
                    span: Span { start, end },
                })
            }
            TokenKind::Symbol("(") => {
                self.bump();
                if self.eat_symbol(")") {
                    let end = self.tokens[self.pos - 1].span.end;
                    return Ok(Expr {
                        kind: ExprKind::Tuple(Vec::new()),
                        span: Span { start, end },
                    });
                }
                let first = self.expr()?;
                if self.eat_symbol(")") {
                    return Ok(first);
                }
                self.symbol(";")?;
                let mut items = vec![first];
                while !self.eat_symbol(")") {
                    items.push(self.expr()?);
                    self.symbol(";")?;
                }
                let end = self.tokens[self.pos - 1].span.end;
                Ok(Expr {
                    kind: ExprKind::Tuple(items),
                    span: Span { start, end },
                })
            }
            _ => self.error("expected expression"),
        }
    }

    fn number_literal(&mut self) -> Result<(Value, Span), ErrorReport> {
        let start = self.current().span.start;
        let negative = self.eat_symbol("-");
        let token = self.bump();
        let span = Span {
            start,
            end: token.span.end,
        };
        match token.kind {
            TokenKind::Int(raw) => {
                let magnitude = raw.parse::<u64>().map_err(|_| {
                    ErrorReport::one(diagnostic(
                        self.source,
                        span,
                        "syntax",
                        "integer literal out of range",
                    ))
                })?;
                let value = if negative && magnitude == (i64::MAX as u64) + 1 {
                    i64::MIN
                } else if magnitude <= i64::MAX as u64 {
                    if negative {
                        -(magnitude as i64)
                    } else {
                        magnitude as i64
                    }
                } else {
                    return Err(ErrorReport::one(diagnostic(
                        self.source,
                        span,
                        "syntax",
                        "integer literal out of range",
                    )));
                };
                Ok((Value::Int(value), span))
            }
            TokenKind::Float(raw) => {
                let mut value = raw.parse::<f64>().map_err(|_| {
                    ErrorReport::one(diagnostic(
                        self.source,
                        span,
                        "syntax",
                        "invalid float literal",
                    ))
                })?;
                if negative {
                    value = -value;
                }
                if !value.is_finite() {
                    return Err(ErrorReport::one(diagnostic(
                        self.source,
                        span,
                        "syntax",
                        "float literal is not finite",
                    )));
                }
                Ok((Value::Float(value), span))
            }
            _ => {
                self.pos -= 1;
                self.error("expected number")
            }
        }
    }
}

fn parse_positive_i64(source: &Source, raw: &str, span: Span) -> Result<i64, ErrorReport> {
    raw.parse().map_err(|_| {
        ErrorReport::one(diagnostic(
            source,
            span,
            "syntax",
            "integer literal out of range (use unary `-` only for negative values)",
        ))
    })
}

fn with_span(value: TyExpr, span: Span) -> TyExpr {
    match value {
        TyExpr::String(_) => TyExpr::String(span),
        TyExpr::Int(_) => TyExpr::Int(span),
        TyExpr::Float(_) => TyExpr::Float(span),
        TyExpr::Bool(_) => TyExpr::Bool(span),
        TyExpr::Any(_) => TyExpr::Any(span),
        TyExpr::Literal(v, _) => TyExpr::Literal(v, span),
        TyExpr::Name(v, _) => TyExpr::Name(v, span),
        TyExpr::Object(v, _) => TyExpr::Object(v, span),
        TyExpr::Map(v, _) => TyExpr::Map(v, span),
        TyExpr::List(v, _) => TyExpr::List(v, span),
        TyExpr::Tuple(v, _) => TyExpr::Tuple(v, span),
        TyExpr::Union(v, _) => TyExpr::Union(v, span),
        TyExpr::Keyed(a, b, _) => TyExpr::Keyed(a, b, span),
        TyExpr::Tagged {
            tag,
            common,
            variants,
            ..
        } => TyExpr::Tagged {
            tag,
            common,
            variants,
            span,
        },
    }
}

fn token_label(kind: &TokenKind) -> String {
    match kind {
        TokenKind::Word(v) | TokenKind::Int(v) | TokenKind::Float(v) => format!("`{v}`"),
        TokenKind::String(_) => "string".into(),
        TokenKind::Symbol(v) => format!("`{v}`"),
        TokenKind::Eof => "end of file".into(),
    }
}

fn antlr_validate(source: &Source, overlay: bool) -> Result<(), ErrorReport> {
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as AntlrParser};

    use crate::generated::oon_lexer::OonLexer;
    use crate::generated::oon_parser::OonParser;

    let mut lexer = OonLexer::new(InputStream::new(&source.text));
    lexer.remove_error_listeners();
    let tokens = CommonTokenStream::new(lexer);
    let mut parser = OonParser::new(tokens);
    parser.remove_error_listeners();
    let parsed = if overlay {
        parser.overlay_document()
    } else {
        parser.schema_document()
    };
    if parsed.is_err() || parser.number_of_syntax_errors() != 0 {
        return Err(ErrorReport::one(diagnostic(
            source,
            Span::default(),
            "syntax",
            "ANTLR parser rejected or recovered the document",
        )));
    }
    Ok(())
}

pub(crate) fn parse_schema(source: &Source) -> Result<SchemaAst, ErrorReport> {
    let ast = Parser::new(source)?.schema()?;
    antlr_validate(source, false)?;
    Ok(ast)
}
pub(crate) fn parse_overlay(source: &Source) -> Result<OverlayAst, ErrorReport> {
    let ast = Parser::new(source)?.overlay()?;
    antlr_validate(source, true)?;
    Ok(ast)
}
