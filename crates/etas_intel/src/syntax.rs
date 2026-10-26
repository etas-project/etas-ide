use crate::service::IntelSemanticToken;
use etas_core::{Span, TextRange};
use etas_frontend::{Frontend, ParseOutput, SourceInput};
use etas_syntax::{TokenKind, ast};

pub const SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "function",
    "type",
    "enumMember",
    "parameter",
    "variable",
    "namespace",
    "property",
    "typeParameter",
    "interface",
    "keyword",
    "string",
    "comment",
    "number",
    "operator",
    "decorator",
];

pub(crate) fn lexical_tokens(parsed: &ParseOutput) -> Vec<IntelSemanticToken> {
    use etas_syntax::{Keyword, Punct};
    let mut previous = None;
    let mut result = Vec::new();
    for token in parsed.parsed.tokens.significant_tokens() {
        let text = &parsed.source.text()
            [token.span.range.start.0 as usize..token.span.range.end.0 as usize];
        let kind = match token.kind {
            TokenKind::Keyword(_) => Some(9),
            TokenKind::StringLit | TokenKind::CharLit => Some(10),
            TokenKind::IntLit | TokenKind::FloatLit => Some(12),
            TokenKind::Ident if previous == Some(TokenKind::Punct(Punct::At)) => Some(14),
            TokenKind::Ident
                if previous == Some(TokenKind::Keyword(Keyword::Perform)) && text == "infer" =>
            {
                Some(9)
            }
            TokenKind::Ident
                if previous == Some(TokenKind::Punct(Punct::Colon))
                    && matches!(text, "trace" | "callable") =>
            {
                Some(9)
            }
            TokenKind::Ident
                if matches!(
                    text,
                    "bool"
                        | "string"
                        | "bytes"
                        | "char"
                        | "unit"
                        | "never"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "isize"
                        | "u8"
                        | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "usize"
                        | "f32"
                        | "f64"
                ) =>
            {
                Some(1)
            }
            TokenKind::Punct(
                Punct::LParen
                | Punct::RParen
                | Punct::LBrace
                | Punct::RBrace
                | Punct::LBracket
                | Punct::RBracket
                | Punct::Comma
                | Punct::Semi
                | Punct::Colon
                | Punct::Dot,
            ) => None,
            TokenKind::Punct(_) => Some(13),
            _ => None,
        };
        if let Some(token_type) = kind {
            result.push(IntelSemanticToken {
                byte_start: token.span.range.start.0,
                byte_end: token.span.range.end.0,
                token_type,
                modifier_bits: 0,
            });
        }
        previous = Some(token.kind.clone());
    }
    for token in &parsed.parsed.tokens.tokens {
        if matches!(token.kind, TokenKind::Comment(_)) {
            result.push(IntelSemanticToken {
                byte_start: token.span.range.start.0,
                byte_end: token.span.range.end.0,
                token_type: 11,
                modifier_bits: 0,
            });
        }
    }
    result.sort_by_key(|token| token.byte_start);
    result
}

pub(crate) fn merge_tokens(
    parsed: &ParseOutput,
    lexical: Vec<IntelSemanticToken>,
    semantic: Vec<IntelSemanticToken>,
) -> Vec<IntelSemanticToken> {
    let identifiers: std::collections::HashSet<_> = parsed
        .parsed
        .tokens
        .tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Ident)
        .map(|token| (token.span.range.start.0, token.span.range.end.0))
        .collect();
    let mut merged = std::collections::BTreeMap::new();
    for token in semantic {
        if identifiers.contains(&(token.byte_start, token.byte_end)) {
            merged.entry(token.byte_start).or_insert(token);
        }
    }
    for token in lexical {
        // Lexical keyword/literal boundaries cannot be painted over by a broad
        // HIR source origin (for example a whole declaration or type expression).
        merged.insert(token.byte_start, token);
    }
    merged.into_values().collect()
}

#[derive(Clone, Copy, Debug)]
pub enum DocumentSymbolKind {
    Module,
    Function,
    Type,
    Enum,
    EnumMember,
    Interface,
    Variable,
}

#[derive(Clone, Debug)]
pub struct DocumentSymbol {
    pub name: String,
    pub kind: DocumentSymbolKind,
    pub range: TextRange,
    pub selection_range: TextRange,
    pub children: Vec<DocumentSymbol>,
}

pub(crate) fn parse(text: &str) -> ParseOutput {
    Frontend.parse(SourceInput::anonymous(text))
}

fn symbol(name: &ast::Name, span: Span, kind: DocumentSymbolKind) -> DocumentSymbol {
    DocumentSymbol {
        name: name.text.clone(),
        kind,
        range: span.range,
        selection_range: name.span.range,
        children: vec![],
    }
}

pub(crate) fn document_symbols(parsed: &ParseOutput) -> Vec<DocumentSymbol> {
    use DocumentSymbolKind as K;
    let program = &parsed.parsed.value;
    let mut result = Vec::new();
    for annotated in &program.items {
        let entry = match &annotated.item {
            ast::Item::Alias(item) => symbol(&item.name, annotated.span, K::Type),
            ast::Item::Type(item) => symbol(&item.name, annotated.span, K::Type),
            ast::Item::Enum(item) => {
                let mut entry = symbol(&item.name, annotated.span, K::Enum);
                entry.children = item
                    .variants
                    .iter()
                    .map(|variant| symbol(&variant.name, variant.span, K::EnumMember))
                    .collect();
                entry
            }
            ast::Item::Spec(item) => {
                let mut entry = symbol(&item.name, annotated.span, K::Interface);
                entry.children = item
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        ast::SpecItem::FlowSignature(flow) => {
                            Some(symbol(&flow.name, flow.span, K::Function))
                        }
                        ast::SpecItem::Error(_) => None,
                    })
                    .collect();
                entry
            }
            ast::Item::Effect(item) => {
                let mut entry = symbol(&item.name, annotated.span, K::Interface);
                entry.children = item
                    .body
                    .actions()
                    .iter()
                    .map(|action| symbol(&action.name, action.span, K::Function))
                    .collect();
                entry
            }
            ast::Item::Flow(item) => symbol(&item.name, annotated.span, K::Function),
            ast::Item::Agent(item) => symbol(&item.name, annotated.span, K::Type),
            ast::Item::Protocol(item) => symbol(&item.name, annotated.span, K::Interface),
            ast::Item::Tool(item) => {
                let Some(name) = item.path.segments.last() else {
                    continue;
                };
                symbol(name, annotated.span, K::Function)
            }
            ast::Item::TopLevelLet(item) => symbol(&item.name, annotated.span, K::Variable),
            ast::Item::Impl(item) => {
                let target = item.target.span();
                DocumentSymbol {
                    name: format!(
                        "impl {}",
                        &parsed.source.text()
                            [target.range.start.0 as usize..target.range.end.0 as usize]
                    ),
                    kind: K::Interface,
                    range: annotated.span.range,
                    selection_range: target.range,
                    children: item
                        .items
                        .iter()
                        .filter_map(|item| match item {
                            ast::ImplItem::Flow(flow) => {
                                Some(symbol(&flow.name, flow.span, K::Function))
                            }
                            ast::ImplItem::Action(action) => {
                                Some(symbol(&action.name, action.span, K::Function))
                            }
                            ast::ImplItem::Error(_) => None,
                        })
                        .collect(),
                }
            }
            ast::Item::Error(_) => continue,
        };
        result.push(entry);
    }
    if let Some(module) = &program.module {
        let mut entry = DocumentSymbol {
            name: module
                .path
                .segments
                .iter()
                .map(|name| name.text.as_str())
                .collect::<Vec<_>>()
                .join("."),
            kind: K::Module,
            range: program.span.range,
            selection_range: module.path.span.range,
            children: result,
        };
        // EOF can follow trailing trivia; the module covers the entire buffer.
        entry.range.end = etas_core::TextSize(parsed.source.text().len() as u32);
        vec![entry]
    } else {
        result
    }
}

pub(crate) fn definition_selection(parsed: &ParseOutput, range: TextRange) -> TextRange {
    fn find(symbols: &[DocumentSymbol], range: TextRange) -> Option<TextRange> {
        for symbol in symbols {
            if symbol.selection_range.start >= range.start
                && symbol.selection_range.end <= range.end
            {
                return Some(symbol.selection_range);
            }
            if let Some(found) = find(&symbol.children, range) {
                return Some(found);
            }
        }
        None
    }
    find(&document_symbols(parsed), range).unwrap_or(range)
}

pub(crate) fn identifier_range(parsed: &ParseOutput, span: Span, name: &str) -> Option<TextRange> {
    parsed
        .parsed
        .tokens
        .tokens
        .iter()
        .find(|token| {
            token.kind == TokenKind::Ident
                && token.span.range.start >= span.range.start
                && token.span.range.end <= span.range.end
                && &parsed.source.text()
                    [token.span.range.start.0 as usize..token.span.range.end.0 as usize]
                    == name
        })
        .map(|token| token.span.range)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_uses_parser_name_spans_and_nested_members() {
        let text = "module sample;\npublic alias Host = string;\nenum State { Ready; Done; }\neffect Read { action read() -> string; }\nflow main() -> i32 { return 0; }";
        let parsed = parse(text);
        assert!(
            parsed.parsed.diagnostics.is_empty(),
            "{:?}",
            parsed.parsed.diagnostics
        );
        let symbols = document_symbols(&parsed);
        assert_eq!(symbols[0].name, "sample");
        assert_eq!(symbols[0].children.len(), 4);
        for symbol in &symbols[0].children {
            assert_eq!(
                &text[symbol.selection_range.start.0 as usize
                    ..symbol.selection_range.end.0 as usize],
                symbol.name
            );
        }
        assert_eq!(symbols[0].children[1].children[0].name, "Ready");
        assert_eq!(symbols[0].children[2].children[0].name, "read");
    }

    #[test]
    fn lexical_tokens_survive_parse_errors_without_coloring_strings_as_keywords() {
        let text = "flow broken( { let text = \"return\"; /* public */ return 42;";
        let parsed = parse(text);
        assert!(!parsed.parsed.diagnostics.is_empty());
        let tokens = lexical_tokens(&parsed);
        let types: Vec<_> = tokens
            .iter()
            .map(|token| {
                (
                    &text[token.byte_start as usize..token.byte_end as usize],
                    token.token_type,
                )
            })
            .collect();
        assert!(types.contains(&("flow", 9)));
        assert!(types.contains(&("\"return\"", 10)));
        assert!(types.contains(&("/* public */", 11)));
        assert!(types.contains(&("42", 12)));
    }
}
