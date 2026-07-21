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

fn byte_col_to_utf16_col(text: &str, line: u32, byte_col: u32) -> u32 {
    let line_start = find_line_start(text, line);
    let line_text = &text[line_start..];
    let end = (byte_col as usize).min(line_text.len());
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
    for t in tokens {
        let pos = line_index.line_col(TextSize(t.byte_start));
        let delta_line = pos.line.wrapping_sub(prev_line);
        let delta_start = if delta_line == 0 {
            pos.col.wrapping_sub(prev_col)
        } else {
            pos.col
        };
        let length = byte_range_utf16_len(source_text, t.byte_start as usize, t.byte_end as usize);
        if length == 0 {
            continue;
        }
        result.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: t.token_type,
            token_modifiers_bitset: t.modifier_bits,
        });
        prev_line = pos.line;
        prev_col = pos.col;
    }
    result
}

fn byte_range_utf16_len(text: &str, start: usize, end: usize) -> u32 {
    let start = start.min(text.len());
    let end = end.min(text.len());
    if start >= end {
        return 0;
    }
    text[start..end].chars().map(|c| c.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

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
