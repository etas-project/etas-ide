use etas_core::{Severity, SourceId, TextRange};

#[derive(Clone, Debug, serde::Serialize)]
pub struct IntelDiagnostic {
    pub source_id: SourceId,
    pub range: TextRange,
    pub severity: IntelSeverity,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum IntelSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl From<Severity> for IntelSeverity {
    fn from(s: Severity) -> Self {
        match s {
            Severity::Error => IntelSeverity::Error,
            Severity::Warning => IntelSeverity::Warning,
            Severity::Info => IntelSeverity::Info,
            Severity::Hint => IntelSeverity::Hint,
        }
    }
}
