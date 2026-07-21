use crate::service::LspPosition;

#[derive(Clone, Debug)]
pub struct CompletionParams {
    pub uri: String,
    pub position: LspPosition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    Keyword,
    Flow,
    Type,
    Enum,
    Spec,
    Agent,
    Tool,
    Policy,
    Protocol,
    Effect,
    Variable,
    Module,
    Field,
}
