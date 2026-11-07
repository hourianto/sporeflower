//! Source-backed diagnostics shared by parsing, binding and name placement.
use crate::{
    Result,
    program::{ClassId, MemberId},
};
use std::{collections::HashMap, fmt, sync::Arc};

/// An authored location. Text and paths are shared by every origin in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingOrigin {
    pub path: Arc<str>,
    pub source: Arc<str>,
    pub line: usize,
    pub column: usize,
    pub width: usize,
}

impl MappingOrigin {
    pub(crate) fn at(&self, line: usize, column: usize, width: usize) -> Self {
        Self {
            line,
            column,
            width,
            ..self.clone()
        }
    }

    pub(crate) fn attach(&self, error: anyhow::Error) -> anyhow::Error {
        if error.is::<MappingDiagnostics>() {
            return error;
        }
        match error.downcast::<MappingDiagnostic>() {
            Ok(mut issue) => {
                if issue.origins.is_empty() {
                    issue.origins.push(self.clone());
                }
                issue.into()
            }
            Err(error) => MappingDiagnostic::new(format!("{error:#}"), self.clone()).into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MappingOrigins {
    pub classes: HashMap<ClassId, MappingOrigin>,
    pub fields: HashMap<MemberId, MappingOrigin>,
    pub methods: HashMap<MemberId, MappingOrigin>,
    pub parameters: HashMap<MemberId, MappingOrigin>,
    pub reserved_classes: HashMap<String, MappingOrigin>,
}

#[derive(Clone, Debug)]
pub struct MappingDiagnostic {
    pub warning: bool,
    pub message: String,
    pub origins: Vec<MappingOrigin>,
    pub notes: Vec<String>,
    pub hint: Option<String>,
}

impl MappingDiagnostic {
    pub fn new(message: impl Into<String>, origin: MappingOrigin) -> Self {
        Self::with_origins(message, [origin])
    }

    pub fn with_origins(
        message: impl Into<String>,
        origins: impl IntoIterator<Item = MappingOrigin>,
    ) -> Self {
        let mut unique: Vec<_> = origins.into_iter().collect();
        unique.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
        unique.dedup();
        Self {
            warning: false,
            message: message.into(),
            origins: unique,
            notes: Vec::new(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn warning(mut self) -> Self {
        self.warning = true;
        self
    }
}

impl fmt::Display for MappingDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{}: {}",
            if self.warning { "warning" } else { "error" },
            self.message
        )?;
        for origin in &self.origins {
            writeln!(f, "  --> {}:{}:{}", origin.path, origin.line, origin.column)?;
            let line = origin
                .source
                .lines()
                .nth(origin.line.saturating_sub(1))
                .unwrap_or("");
            let prefix: String = line
                .chars()
                .take(origin.column.saturating_sub(1))
                .map(|ch| if ch == '\t' { '\t' } else { ' ' })
                .collect();
            let width = origin
                .width
                .min(
                    line.chars()
                        .count()
                        .saturating_sub(origin.column.saturating_sub(1)),
                )
                .max(1);
            writeln!(f, "{:>4} | {line}", origin.line)?;
            writeln!(f, "     | {prefix}{}", "^".repeat(width))?;
        }
        for note in &self.notes {
            writeln!(f, "  = {note}")?;
        }
        if let Some(hint) = &self.hint {
            writeln!(f, "  hint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MappingDiagnostic {}

/// A deterministic batch of independent errors; also available through anyhow downcasting.
#[derive(Clone, Debug, Default)]
pub struct MappingDiagnostics {
    pub issues: Vec<MappingDiagnostic>,
}

impl MappingDiagnostics {
    pub(crate) fn push(&mut self, error: anyhow::Error) {
        let error = match error.downcast::<Self>() {
            Ok(batch) => {
                self.issues.extend(batch.issues);
                return;
            }
            Err(error) => error,
        };
        self.issues.push(
            error
                .downcast::<MappingDiagnostic>()
                .unwrap_or_else(|error| MappingDiagnostic {
                    warning: false,
                    message: format!("{error:#}"),
                    origins: Vec::new(),
                    notes: Vec::new(),
                    hint: None,
                }),
        );
    }

    pub(crate) fn finish(self) -> Result<()> {
        if self.issues.is_empty() {
            Ok(())
        } else {
            Err(self.into())
        }
    }
}

impl fmt::Display for MappingDiagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, issue) in self.issues.iter().enumerate() {
            if index != 0 {
                writeln!(f)?;
            }
            write!(f, "{issue}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MappingDiagnostics {}
