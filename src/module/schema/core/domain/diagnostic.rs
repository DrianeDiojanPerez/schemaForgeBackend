use std::fmt;

use super::entity::Position;

/// Stable diagnostic codes. Tests and the canvas both key off these rather than
/// off the message text, so wording can be reworded without breaking either.
///
/// The five structural defect classes the validation engine detects are
/// DUPLICATE_NAME, MISSING_PRIMARY_KEY, INVALID_FOREIGN_KEY,
/// CIRCULAR_DEPENDENCY, and TYPE_MISMATCH. The rest are well-formedness checks
/// on the diagram itself.
pub mod code {
    // Visual syntax: the diagram is not well formed.
    pub const DANGLING_RELATIONSHIP: &str = "SF-REL-DANGLING";
    pub const ATTRIBUTE_WITHOUT_TYPE: &str = "SF-ATTR-UNTYPED";
    pub const EMPTY_NAME: &str = "SF-NAME-EMPTY";
    pub const EMPTY_SCHEMA: &str = "SF-SCHEMA-EMPTY";

    // Structural defects: the schema is not internally consistent.
    pub const DUPLICATE_ENTITY_NAME: &str = "SF-DUP-ENTITY";
    pub const DUPLICATE_ATTRIBUTE_NAME: &str = "SF-DUP-ATTRIBUTE";
    pub const MISSING_PRIMARY_KEY: &str = "SF-KEY-MISSING";
    pub const INVALID_FOREIGN_KEY: &str = "SF-FK-INVALID";
    pub const FOREIGN_KEY_NOT_A_KEY: &str = "SF-FK-NOT-A-KEY";
    pub const CIRCULAR_DEPENDENCY: &str = "SF-DEP-CYCLE";
    pub const TYPE_MISMATCH: &str = "SF-TYPE-MISMATCH";

    // Soft, non-blocking.
    pub const MISSING_DESCRIPTION: &str = "SF-DOC-MISSING";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A structured diagnostic rather than free text, so one value drives both the
/// inline display on the canvas and an assertion in an automated test.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    /// Ids of the offending elements: entity, attribute, or relationship.
    pub element_ids: Vec<String>,
    pub location: Option<Position>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            element_ids: Vec::new(),
            location: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            message: message.into(),
            element_ids: Vec::new(),
            location: None,
        }
    }

    pub fn about(mut self, element_id: impl Into<String>) -> Self {
        self.element_ids.push(element_id.into());
        self
    }

    pub fn at(mut self, location: Position) -> Self {
        self.location = Some(location);
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// The outcome of a validation run. A schema is valid when nothing rose above a
/// warning, so a missing description never blocks generation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
}

impl Report {
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self { diagnostics }
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn is_valid(&self) -> bool {
        !self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| !d.is_error())
    }

    pub fn has(&self, code: &str) -> bool {
        self.diagnostics.iter().any(|d| d.code == code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_report_is_valid() {
        assert!(Report::default().is_valid());
    }

    #[test]
    fn a_warning_alone_does_not_invalidate_a_schema() {
        let report = Report::new(vec![Diagnostic::warning(
            code::MISSING_DESCRIPTION,
            "entity `users` has no description",
        )
        .about("e1")]);

        assert!(
            report.is_valid(),
            "undocumented is not the same as incorrect"
        );
        assert_eq!(report.warnings().count(), 1);
        assert_eq!(report.errors().count(), 0);
    }

    #[test]
    fn a_single_error_invalidates_the_schema() {
        let report = Report::new(vec![
            Diagnostic::warning(code::MISSING_DESCRIPTION, "undocumented"),
            Diagnostic::error(
                code::MISSING_PRIMARY_KEY,
                "entity `logs` has no primary key",
            )
            .about("e9"),
        ]);

        assert!(!report.is_valid());
        assert!(report.has(code::MISSING_PRIMARY_KEY));
        assert!(!report.has(code::TYPE_MISMATCH));
    }

    #[test]
    fn a_diagnostic_carries_every_element_it_blames() {
        let diagnostic = Diagnostic::error(code::TYPE_MISMATCH, "type mismatch")
            .about("e2")
            .about("a3")
            .at(Position::new(120.0, 80.0));

        assert_eq!(diagnostic.element_ids, vec!["e2", "a3"]);
        assert_eq!(diagnostic.location, Some(Position::new(120.0, 80.0)));
    }
}
