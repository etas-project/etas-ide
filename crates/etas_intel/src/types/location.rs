use etas_core::TextRange;

#[derive(Clone, Debug)]
pub struct IntelLocation {
    pub uri: String,
    pub range: TextRange,
    pub source_text: String,
}
