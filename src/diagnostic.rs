// SPDX-License-Identifier: MPL-2.0

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub source: String,
    pub line: usize,
    pub column: usize,
    pub phase: String,
    pub message: String,
    pub overlay: Option<String>,
    pub declaration: Option<String>,
    pub path: Option<String>,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}: {}",
            self.source, self.line, self.column, self.phase, self.message
        )?;
        if let Some(value) = &self.overlay {
            write!(f, " [overlay {value}]")?;
        }
        if let Some(value) = &self.declaration {
            write!(f, " [declaration {value}]")?;
        }
        if let Some(value) = &self.path {
            write!(f, " [path {value}]")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorReport {
    pub diagnostics: Vec<Diagnostic>,
}

impl ErrorReport {
    pub(crate) fn one(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }
}

impl fmt::Display for ErrorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index != 0 {
                writeln!(f)?;
            }
            write!(f, "{diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ErrorReport {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
}

pub(crate) fn diagnostic(
    source: &crate::Source,
    span: Span,
    phase: &str,
    message: impl Into<String>,
) -> Diagnostic {
    let prefix = &source.text.as_bytes()[..span.start.min(source.text.len())];
    let line = prefix.iter().filter(|byte| **byte == b'\n').count() + 1;
    let line_start = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let column = source.text[line_start..span.start.min(source.text.len())]
        .chars()
        .count()
        + 1;
    Diagnostic {
        source: source.name.clone(),
        line,
        column,
        phase: phase.to_owned(),
        message: message.into(),
        overlay: None,
        declaration: None,
        path: None,
    }
}
