use tower_lsp::lsp_types;

use etas_core::{LineIndex, TextRange, TextSize};
use etas_intel::{
    CompletionItem, CompletionKind, IntelDiagnostic, IntelLocation, IntelSemanticToken,
    IntelSeverity,
};

pub fn intel_diagnostic_to_lsp(diag: &IntelDiagnostic, source_text: &str) -> lsp_types::Diagnostic {
    lsp_types::Diagnostic {
        range: text_range_to_lsp_range(&diag.range, source_text),
        severity: Some(intel_severity_to_lsp(diag.severity)),
        code: Some(lsp_types::NumberOrString::String(diag.code.clone())),
        source: Some("etas".to_string()),
        message: diag.message.clone(),
        related_information: None,
        tags: None,
        code_description: None,
        data: None,
    }
}

fn intel_severity_to_lsp(s: IntelSeverity) -> lsp_types::DiagnosticSeverity {
    match s {
        IntelSeverity::Error => lsp_types::DiagnosticSeverity::ERROR,
        IntelSeverity::Warning => lsp_types::DiagnosticSeverity::WARNING,
        IntelSeverity::Info => lsp_types::DiagnosticSeverity::INFORMATION,
        IntelSeverity::Hint => lsp_types::DiagnosticSeverity::HINT,
    }
}

fn text_range_to_lsp_range(range: &TextRange, source_text: &str) -> lsp_types::Range {
    let line_index = LineIndex::new(source_text);
    let start = line_index.line_col(range.start);
    let end = line_index.line_col(range.end);

    lsp_types::Range {
        start: lsp_types::Position {
            line: start.line,
            character: byte_col_to_utf16_col(source_text, start.line, start.col),
        },
        end: lsp_types::Position {
            line: end.line,
            character: byte_col_to_utf16_col(source_text, end.line, end.col),
        },
    }
}

pub fn intel_symbol_to_lsp(
    symbol: &etas_intel::DocumentSymbol,
    text: &str,
) -> lsp_types::DocumentSymbol {
    use etas_intel::DocumentSymbolKind as K;
    #[allow(deprecated)]
    lsp_types::DocumentSymbol {
        name: symbol.name.clone(),
        detail: None,
        kind: match symbol.kind {
            K::Module => lsp_types::SymbolKind::MODULE,
            K::Function => lsp_types::SymbolKind::FUNCTION,
            K::Type => lsp_types::SymbolKind::STRUCT,
            K::Enum => lsp_types::SymbolKind::ENUM,
            K::EnumMember => lsp_types::SymbolKind::ENUM_MEMBER,
            K::Interface => lsp_types::SymbolKind::INTERFACE,
            K::Variable => lsp_types::SymbolKind::VARIABLE,
        },
        tags: None,
        deprecated: None,
        range: text_range_to_lsp_range(&symbol.range, text),
        selection_range: text_range_to_lsp_range(&symbol.selection_range, text),
        children: Some(
            symbol
                .children
                .iter()
                .map(|child| intel_symbol_to_lsp(child, text))
                .collect(),
        ),
    }
}

fn byte_col_to_utf16_col(text: &str, line: u32, byte_col: u32) -> u32 {
    let line_start = find_line_start(text, line);
    let line_text = &text[line_start..];
    let mut end = (byte_col as usize).min(line_text.find('\n').unwrap_or(line_text.len()));
    while !line_text.is_char_boundary(end) {
        end -= 1;
    }
    line_text[..end].chars().map(|c| c.len_utf16() as u32).sum()
}

fn find_line_start(text: &str, target_line: u32) -> usize {
    let mut current_line = 0u32;
    for (i, ch) in text.char_indices() {
        if current_line == target_line {
            return i;
        }
        if ch == '\n' {
            current_line += 1;
        }
    }
    text.len()
}

pub fn intel_location_to_lsp(loc: &IntelLocation) -> lsp_types::Location {
    let uri = lsp_types::Url::parse(&loc.uri)
        .unwrap_or_else(|_| lsp_types::Url::parse("file:///unknown.es").unwrap());
    let range = text_range_to_lsp_range(&loc.range, &loc.source_text);
    lsp_types::Location { uri, range }
}

pub fn intel_completion_to_lsp(item: CompletionItem) -> lsp_types::CompletionItem {
    lsp_types::CompletionItem {
        label: item.label,
        kind: Some(completion_kind_to_lsp(item.kind)),
        detail: item.detail,
        insert_text: item.insert_text,
        ..Default::default()
    }
}

fn completion_kind_to_lsp(kind: CompletionKind) -> lsp_types::CompletionItemKind {
    match kind {
        CompletionKind::Keyword => lsp_types::CompletionItemKind::KEYWORD,
        CompletionKind::Flow | CompletionKind::Agent | CompletionKind::Tool => {
            lsp_types::CompletionItemKind::FUNCTION
        }
        CompletionKind::Type | CompletionKind::Enum | CompletionKind::Protocol => {
            lsp_types::CompletionItemKind::CLASS
        }
        CompletionKind::Policy | CompletionKind::Effect | CompletionKind::Spec => {
            lsp_types::CompletionItemKind::INTERFACE
        }
        CompletionKind::Variable => lsp_types::CompletionItemKind::VARIABLE,
        CompletionKind::Module => lsp_types::CompletionItemKind::MODULE,
        CompletionKind::Field => lsp_types::CompletionItemKind::FIELD,
    }
}

pub fn intel_tokens_to_lsp(
    tokens: &[IntelSemanticToken],
    source_text: &str,
) -> Vec<lsp_types::SemanticToken> {
    use lsp_types::SemanticToken;
    let line_index = LineIndex::new(source_text);
    let mut result = Vec::new();
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;
    let mut sorted: Vec<_> = tokens.iter().collect();
    sorted.sort_by_key(|token| (token.byte_start, token.byte_end));
    let mut previous_end = 0;
    for t in sorted {
        let start = t.byte_start as usize;
        let end = t.byte_end as usize;
        if start < previous_end
            || start >= end
            || end > source_text.len()
            || !source_text.is_char_boundary(start)
            || !source_text.is_char_boundary(end)
        {
            continue;
        }
        previous_end = end;
        let mut offset = start;
        for part in source_text[start..end].split_inclusive('\n') {
            let content = part.trim_end_matches(['\r', '\n']);
            if !content.is_empty() {
                let pos = line_index.line_col(TextSize(offset as u32));
                let col = byte_col_to_utf16_col(source_text, pos.line, pos.col);
                let delta_line = pos.line - prev_line;
                result.push(SemanticToken {
                    delta_line,
                    delta_start: if delta_line == 0 { col - prev_col } else { col },
                    length: content.encode_utf16().count() as u32,
                    token_type: t.token_type,
                    token_modifiers_bitset: t.modifier_bits,
                });
                prev_line = pos.line;
                prev_col = col;
            }
            offset += part.len();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_tokens_use_utf16_columns_and_split_multiline_tokens() {
        let text = "\"中文😀\" return\r\n/* one\r\n中文 */ let";
        let token = |start, end, ty| IntelSemanticToken {
            byte_start: start,
            byte_end: end,
            token_type: ty,
            modifier_bits: 0,
        };
        let ret = text.find("return").unwrap() as u32;
        let comment = text.find("/*").unwrap() as u32;
        let comment_end = text.find("*/").unwrap() as u32 + 2;
        let let_start = text.find("let").unwrap() as u32;
        let tokens = intel_tokens_to_lsp(
            &[
                token(0, ret - 1, 10),
                token(ret, ret + 6, 9),
                token(comment, comment_end, 11),
                token(let_start, let_start + 3, 9),
            ],
            text,
        );
        let positions: Vec<_> = tokens
            .iter()
            .map(|token| (token.delta_line, token.delta_start, token.length))
            .collect();
        assert_eq!(
            positions,
            [(0, 0, 6), (0, 7, 6), (1, 0, 6), (1, 0, 5), (0, 6, 3)]
        );
    }

    #[test]
    fn token_conversion_rejects_invalid_boundaries_and_overlap() {
        let tokens = [
            IntelSemanticToken {
                byte_start: 1,
                byte_end: 2,
                token_type: 0,
                modifier_bits: 0,
            },
            IntelSemanticToken {
                byte_start: 0,
                byte_end: 3,
                token_type: 1,
                modifier_bits: 0,
            },
            IntelSemanticToken {
                byte_start: 0,
                byte_end: 999,
                token_type: 1,
                modifier_bits: 0,
            },
        ];
        let converted = intel_tokens_to_lsp(&tokens, "中");
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].length, 1);
    }

    #[test]
    fn maps_completion_kinds_to_lsp_kinds() {
        let cases = [
            (
                CompletionKind::Keyword,
                lsp_types::CompletionItemKind::KEYWORD,
            ),
            (
                CompletionKind::Flow,
                lsp_types::CompletionItemKind::FUNCTION,
            ),
            (
                CompletionKind::Agent,
                lsp_types::CompletionItemKind::FUNCTION,
            ),
            (
                CompletionKind::Tool,
                lsp_types::CompletionItemKind::FUNCTION,
            ),
            (CompletionKind::Type, lsp_types::CompletionItemKind::CLASS),
            (
                CompletionKind::Module,
                lsp_types::CompletionItemKind::MODULE,
            ),
            (CompletionKind::Field, lsp_types::CompletionItemKind::FIELD),
        ];

        for (intel, lsp) in cases {
            assert_eq!(completion_kind_to_lsp(intel), lsp);
        }
    }
}
