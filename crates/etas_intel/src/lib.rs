mod completion;
pub mod service;
pub mod types;

pub use service::{
    Command, Event, HoverResult, IntelSemanticToken, LspPosition, LspRange, LspTextEdit,
    spawn_service, spawn_service_with_events,
};
pub use types::completion::{CompletionItem, CompletionKind, CompletionParams};
pub use types::diagnostic::{IntelDiagnostic, IntelSeverity};
pub use types::location::IntelLocation;
