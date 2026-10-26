use super::*;
use crate::{CompletionKind, IntelSeverity};

fn open_and_check(
    tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    uri: &str,
    text: &str,
) -> Vec<IntelDiagnostic> {
    let _ = tx.send(Command::OpenDocument {
        uri: uri.to_string(),
        text: text.to_string(),
        version: 1,
    });

    std::thread::sleep(std::time::Duration::from_millis(50));

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GetDiagnostics {
        respond: respond_tx,
    });
    let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
    diags.get(uri).cloned().unwrap_or_default()
}

fn goto_def(
    tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    uri: &str,
    line: u32,
    character: u32,
) -> Vec<IntelLocation> {
    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GoToDefinition {
        uri: uri.to_string(),
        position: LspPosition { line, character },
        respond: respond_tx,
    });
    respond_rx.blocking_recv().unwrap_or_default()
}

fn completion(
    tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    uri: &str,
    line: u32,
    character: u32,
) -> Vec<CompletionItem> {
    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::Completion {
        params: CompletionParams {
            uri: uri.to_string(),
            position: LspPosition { line, character },
        },
        respond: respond_tx,
    });
    respond_rx.blocking_recv().unwrap_or_default()
}

fn completion_at_marker(
    tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    uri: &str,
    source: &str,
    marker: &str,
) -> Vec<CompletionItem> {
    let (text, line, character) = source_without_marker_and_position(source, marker);

    open_and_check(tx, uri, &text);
    completion(tx, uri, line, character)
}

fn source_without_marker_and_position(source: &str, marker: &str) -> (String, u32, u32) {
    let offset = source.find(marker).expect("test marker must exist");
    let mut text = source.to_string();
    text.replace_range(offset..offset + marker.len(), "");

    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let character = before
        .rsplit_once('\n')
        .map(|(_, line)| line.chars().count())
        .unwrap_or_else(|| before.chars().count()) as u32;
    (text, line, character)
}

fn project_completion_at_marker(
    tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    root: &std::path::Path,
    app_path: &std::path::Path,
    app_source: &str,
    marker: &str,
) -> Vec<CompletionItem> {
    let (text, line, character) = source_without_marker_and_position(app_source, marker);
    std::fs::write(app_path, &text).unwrap();
    let uri = format!("file://{}", app_path.display());
    let _ = tx.send(Command::SetWorkspaceRoot(root.to_path_buf()));
    std::thread::sleep(std::time::Duration::from_millis(10));
    open_file(tx, &uri, &text);
    completion(tx, &uri, line, character)
}

fn open_file(tx: &tokio::sync::mpsc::UnboundedSender<Command>, uri: &str, text: &str) {
    let _ = tx.send(Command::OpenDocument {
        uri: uri.to_string(),
        text: text.to_string(),
        version: 1,
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
}

fn hover_at(
    tx: &tokio::sync::mpsc::UnboundedSender<Command>,
    uri: &str,
    source: &str,
    needle: &str,
) -> Option<HoverResult> {
    let offset = source.find(needle).expect("hover marker must exist");
    let before = &source[..offset];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let character = before.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
    let (respond, rx) = tokio::sync::oneshot::channel();
    tx.send(Command::Hover {
        uri: uri.to_owned(),
        position: LspPosition { line, character },
        respond,
    })
    .unwrap();
    rx.blocking_recv().unwrap()
}

fn assert_hover(result: Option<HoverResult>, expected: &str) -> HoverResult {
    let result = result.expect("expected checked hover");
    assert!(
        result.text.contains(expected),
        "expected {expected:?}, got {}",
        result.text
    );
    for internal in [
        "TypeId(",
        "SymbolId(",
        "FlowType",
        "Function(",
        "Primitive(",
    ] {
        assert!(
            !result.text.contains(internal),
            "hover leaked {internal}: {}",
            result.text
        );
    }
    result
}

fn assert_checked(tx: &tokio::sync::mpsc::UnboundedSender<Command>, uri: &str, source: &str) {
    let diagnostics = open_and_check(tx, uri, source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn test_hover_callable_declarations_calls_parameters_and_locals() {
    let tx = spawn_service();
    let uri = "file:///hover_signature.es";
    let source = "module hover;\nflow square(x: i32) -> i32 { return x * x; }\nflow main() -> i32 { let answer = square(4); return answer; }\n";
    assert_checked(&tx, uri, source);
    for needle in ["square(x", "square(4"] {
        let result = assert_hover(
            hover_at(&tx, uri, source, needle),
            "flow square(x: i32) -> i32",
        );
        assert_eq!(&source[result.byte_start..result.byte_end], "square");
    }
    assert_hover(hover_at(&tx, uri, source, "x: i32"), "x: i32");
    assert_hover(hover_at(&tx, uri, source, "x * x"), "x: i32");
    assert_hover(hover_at(&tx, uri, source, "answer ="), "let answer: i32");
    assert_hover(hover_at(&tx, uri, source, "answer;"), "let answer: i32");
    assert!(hover_at(&tx, uri, source, "return x").is_none());
}

#[test]
fn test_hover_generic_spec_bounds_and_nominal_identity() {
    let tx = spawn_service();
    let uri = "file:///hover_generics.es";
    let source = "module hover;\nspec Region;\ntype Root;\nimpl Root ~ Region;\ntype Path<R ~ Region> = string;\nflow identity<R ~ Region>(path: Path<R>) -> Path<R> ![] { return path; }\nflow main() -> i32 { return 0; }\n";
    assert_checked(&tx, uri, source);
    assert_hover(
        hover_at(&tx, uri, source, "identity<R"),
        "flow identity<R ~ Region>(path: hover.Path<R>) -> hover.Path<R> ![]",
    );
    assert_hover(hover_at(&tx, uri, source, "path;"), "path: hover.Path<R>");
}

#[test]
fn test_hover_alias_nominal_fields_and_nested_types() {
    let tx = spawn_service();
    let uri = "file:///hover_types.es";
    let source = "module hover;\nalias Label = string;\ntype UserId = string;\nalias User = { id: UserId, name: Label };\nflow label(user: User, values: Array<Option<i32>>) -> string ![] { return user.name; }\nflow main() -> i32 { return 0; }\n";
    assert_checked(&tx, uri, source);
    assert_hover(
        hover_at(&tx, uri, source, "Label ="),
        "alias Label = string",
    );
    assert_hover(
        hover_at(&tx, uri, source, "UserId ="),
        "type UserId = string",
    );
    assert_hover(
        hover_at(&tx, uri, source, "values:"),
        "values: Array<Option<i32>>",
    );
    assert_hover(hover_at(&tx, uri, source, "name;"), "name: string");
    assert_hover(hover_at(&tx, uri, source, "id: UserId"), "id: hover.UserId");
}

#[test]
fn test_hover_effect_contract_and_requested_actions() {
    let tx = spawn_service();
    let uri = "file:///hover_effects.es";
    let source = "module hover;\nspec Region;\ntype Root;\nimpl Root ~ Region;\neffect Store extends FileIO { action read<R ~ Region>(key: string) -> string; }\nflow fetch<R ~ Region>(key: string) -> string ![Store.read<R>] { return perform Store.read<R>(key); }\nflow main() -> i32 { return 0; }\n";
    assert_checked(&tx, uri, source);
    let result = assert_hover(
        hover_at(&tx, uri, source, "fetch<R"),
        "flow fetch<R ~ Region>(key: string) -> string ![Store.read<R>]",
    );
    assert!(
        result.text.contains("Requested actions:"),
        "{}",
        result.text
    );
    assert!(
        result.text.matches("Store.read<R>").count() >= 2,
        "{}",
        result.text
    );
}

#[test]
fn test_hover_tool_agent_and_function_value() {
    let tx = spawn_service();
    let uri = "file:///hover_callables.es";
    let source = "module hover;\ntool echo(value: string) -> string ![] { return value; }\nagent Greeter(input: string) -> string {}\nflow square(x: i32) -> i32 ![] { return x * x; }\nflow main() -> i32 { let callback = square; return callback(2); }\n";
    assert_checked(&tx, uri, source);
    assert_hover(
        hover_at(&tx, uri, source, "echo(value"),
        "tool echo(value: string) -> string ![]",
    );
    assert_hover(
        hover_at(&tx, uri, source, "Greeter(input"),
        "agent Greeter(input: string) -> string",
    );
    assert_hover(
        hover_at(&tx, uri, source, "callback(2"),
        "let callback: (i32) -> i32 ![]",
    );
}

#[test]
fn test_hover_preserves_higher_order_latent_effects() {
    let tx = spawn_service();
    let uri = "file:///hover_latent_effects.es";
    let source = "module hover;\nflow twice<effect E>(callback: () -> unit ![E]) -> unit ![E] { callback(); callback(); }\nflow main() -> i32 { return 0; }\n";
    assert_checked(&tx, uri, source);
    assert_hover(
        hover_at(&tx, uri, source, "twice<effect"),
        "flow twice<effect E>(callback: () -> unit ![E]) -> unit ![E]",
    );
}

#[test]
fn test_hover_uses_utf16_positions_and_refuses_stale_snapshot() {
    let mut service = LanguageService::new();
    let uri = "file:///hover_stale.es";
    let source =
        "module hover;\nflow main() -> i32 { let text = \"😀\"; let result = 1; return result; }\n";
    service.open_document(uri.to_owned(), source.to_owned(), 1);
    service.ensure_fresh_analysis();
    let offset = source.rfind("result;").unwrap();
    let position = LspPosition {
        line: 1,
        character: source.split('\n').nth(1).unwrap()[..offset - source.find('\n').unwrap() - 1]
            .encode_utf16()
            .count() as u32,
    };
    let result = assert_hover(service.hover(uri, position), "let result: i32");
    assert_eq!(&source[result.byte_start..result.byte_end], "result");
    service.change_document(
        uri.to_owned(),
        vec![LspTextEdit {
            range: None,
            text: source.replace("result;", "missing;"),
        }],
        2,
    );
    service.ensure_fresh_analysis();
    assert!(
        service.hover(uri, position).is_none(),
        "old facts must not describe the new identifier"
    );
}

#[test]
fn test_hover_resolves_import_alias_and_wildcard_calls() {
    let tx = spawn_service();
    let root = std::env::temp_dir().join(format!("etas_hover_imports_{}", std::process::id()));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("math.es"),
        "module math;\npublic flow square(x: i32) -> i32 ![] { return x * x; }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("utils.es"),
        "module utils;\npublic flow add(a: i32, b: i32) -> i32 ![] { return a + b; }\n",
    )
    .unwrap();
    let source = "module main;\nimport math.square as squared;\nimport utils.*;\nflow main() -> i32 { return add(squared(2), 3); }\n";
    let file = src.join("main.es");
    std::fs::write(&file, source).unwrap();
    let uri = path_to_uri(&file);
    tx.send(Command::SetWorkspaceRoot(root.clone())).unwrap();
    assert_checked(&tx, &uri, source);
    assert_hover(
        hover_at(&tx, &uri, source, "squared(2"),
        "flow square(x: i32) -> i32 ![]",
    );
    assert_hover(
        hover_at(&tx, &uri, source, "add(squared"),
        "flow add(a: i32, b: i32) -> i32 ![]",
    );
    drop(tx);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_offset_of_position_clamps_to_current_line() {
    let text = "abc\ndef\n";
    let offset = offset_of_position(
        text,
        LspPosition {
            line: 0,
            character: 20,
        },
    );
    assert_eq!(offset, 3, "offset should clamp to first line end");

    let unicode = "a😀\nz";
    let offset = offset_of_position(
        unicode,
        LspPosition {
            line: 0,
            character: 3,
        },
    );
    assert_eq!(offset, "a😀".len(), "LSP columns are UTF-16 based");
}

#[test]
fn test_completion_returns_keywords_in_empty_file() {
    let tx = spawn_service();
    let uri = "file:///empty_completion.es";
    open_file(&tx, uri, "");

    let items = completion(&tx, uri, 0, 0);

    assert!(
        items
            .iter()
            .any(|item| item.label == "flow" && item.kind == CompletionKind::Keyword),
        "expected keyword completion, got: {:?}",
        items
    );
}

#[test]
fn test_completion_filters_expression_and_type_positions() {
    let tx = spawn_service();

    let source = r#"module test;

type UserId = string;

flow helper() -> i32 {
    return 1;
}

flow main(value: UserId) -> i32 {
    return helper();
}
"#;

    let uri = "file:///completion_symbols.es";
    open_and_check(&tx, uri, source);

    let expression_items = completion(&tx, uri, 9, 11);

    assert!(
        expression_items
            .iter()
            .any(|item| item.label == "helper" && item.kind == CompletionKind::Flow),
        "expected flow completion in expression context, got: {:?}",
        expression_items
    );
    assert!(
        expression_items.iter().any(|item| item.label == "helper"
            && item.kind == CompletionKind::Flow
            && item.detail.as_deref() == Some("helper() -> i32")),
        "expected flow signature detail, got: {:?}",
        expression_items
    );
    assert!(
        expression_items
            .iter()
            .all(|item| !(item.label == "UserId" && item.kind == CompletionKind::Type)),
        "did not expect type completion in expression context, got: {:?}",
        expression_items
    );

    let type_items = completion(&tx, uri, 8, 17);

    assert!(
        type_items
            .iter()
            .any(|item| item.label == "UserId" && item.kind == CompletionKind::Type),
        "expected type completion in type context, got: {:?}",
        type_items
    );
    assert!(
        type_items
            .iter()
            .any(|item| item.label == "i32" && item.kind == CompletionKind::Type),
        "expected primitive type completion, got: {:?}",
        type_items
    );
    assert!(
        type_items
            .iter()
            .all(|item| !(item.label == "helper" && item.kind == CompletionKind::Flow)),
        "did not expect flow completion in type context, got: {:?}",
        type_items
    );
}

#[test]
fn test_completion_filters_top_level_keywords() {
    let tx = spawn_service();

    let source = r#"module test;

flow helper() -> i32 {
    return 1;
}

"#;

    let uri = "file:///completion_top_level.es";
    open_and_check(&tx, uri, source);

    let items = completion(&tx, uri, 6, 0);

    assert!(
        items
            .iter()
            .any(|item| item.label == "flow" && item.kind == CompletionKind::Keyword),
        "expected top-level declaration keyword, got: {:?}",
        items
    );
    assert!(
        items
            .iter()
            .all(|item| !(item.label == "return" && item.kind == CompletionKind::Keyword)),
        "did not expect expression keyword at top level, got: {:?}",
        items
    );
    assert!(
        items
            .iter()
            .all(|item| !(item.label == "helper" && item.kind == CompletionKind::Flow)),
        "did not expect project symbols at top level, got: {:?}",
        items
    );
}

#[test]
fn test_completion_top_level_keywords_include_new_syntax_forms() {
    let tx = spawn_service();

    let source = r#"module test;

flow helper() -> i32 {
    return 1;
}

"#;

    let uri = "file:///completion_top_level_new.es";
    open_and_check(&tx, uri, source);

    let items = completion(&tx, uri, 6, 0);

    assert!(
        items
            .iter()
            .any(|item| item.label == "alias" && item.kind == CompletionKind::Keyword),
        "expected alias keyword, got: {:?}",
        items
    );
    assert!(
        items
            .iter()
            .any(|item| item.label == "spec" && item.kind == CompletionKind::Keyword),
        "expected spec keyword, got: {:?}",
        items
    );
    assert!(
        items
            .iter()
            .any(|item| item.label == "impl" && item.kind == CompletionKind::Keyword),
        "expected impl keyword, got: {:?}",
        items
    );
}

#[test]
fn test_completion_maps_new_syntax_symbol_kinds() {
    let tx = spawn_service();

    let source = r#"module test;

alias UserName = string;

spec ByteStream;

flow main(value: i32) -> i32 {
    return 0;
}
"#;

    let uri = "file:///completion_new_syntax_symbols.es";
    open_and_check(&tx, uri, source);

    let items = completion(&tx, uri, 6, 18);

    assert!(
        items
            .iter()
            .any(|item| item.label == "UserName" && item.kind == CompletionKind::Type),
        "expected type alias completion, got: {:?}",
        items
    );
    assert!(
        items
            .iter()
            .any(|item| item.label == "ByteStream" && item.kind == CompletionKind::Spec),
        "expected spec completion, got: {:?}",
        items
    );
}

#[test]
fn test_completion_maps_additional_top_level_symbol_kinds() {
    let tx = spawn_service();

    let source = r#"module test;

type Draft = string;

agent Greeter(input: Draft) -> Draft {}

tool normalize(input: string) -> string {
    return "ok";
}

flow main() -> i32 {
    return 0;
}
"#;

    let uri = "file:///completion_extra_symbols.es";
    open_and_check(&tx, uri, source);

    let items = completion(&tx, uri, 11, 11);

    assert!(
        items
            .iter()
            .any(|item| item.label == "Greeter" && item.kind == CompletionKind::Agent),
        "expected agent completion, got: {:?}",
        items
    );
    assert!(
        items.iter().any(|item| item.label == "Greeter"
            && item.kind == CompletionKind::Agent
            && item.detail.as_deref() == Some("Greeter(test.Draft) -> test.Draft")),
        "expected agent signature detail, got: {:?}",
        items
    );
    assert!(
        items
            .iter()
            .any(|item| item.label == "normalize" && item.kind == CompletionKind::Tool),
        "expected tool completion, got: {:?}",
        items
    );
    assert!(
        items.iter().any(|item| item.label == "normalize"
            && item.kind == CompletionKind::Tool
            && item.detail.as_deref() == Some("normalize(string) -> string")),
        "expected tool signature detail, got: {:?}",
        items
    );
}

#[test]
fn test_completion_includes_visible_params_and_locals() {
    let tx = spawn_service();

    let source = r#"module test;

flow main(value: i32) -> i32 {
    let outer = value;
    return outer$CURSOR$;
}
"#;

    let items = completion_at_marker(
        &tx,
        "file:///completion_scope_locals.es",
        source,
        "$CURSOR$",
    );

    assert!(
        items.iter().any(|item| item.label == "value"
            && item.kind == CompletionKind::Variable
            && item.detail.as_deref() == Some("parameter")),
        "expected parameter completion, got: {:?}",
        items
    );
    assert!(
        items.iter().any(|item| item.label == "outer"
            && item.kind == CompletionKind::Variable
            && item.detail.as_deref() == Some("local")),
        "expected local completion, got: {:?}",
        items
    );
}

#[test]
fn test_completion_respects_nested_block_scope() {
    let tx = spawn_service();

    let source = r#"module test;

flow main(value: i32) -> i32 {
    let outer = value;
    if true {
        let inner = outer;
        return 1;
    }
    return outer$OUTER_CURSOR$;
}
"#;

    let outer_items = completion_at_marker(
        &tx,
        "file:///completion_nested_scope.es",
        source,
        "$OUTER_CURSOR$",
    );

    assert!(
        outer_items.iter().any(|item| item.label == "outer"
            && item.kind == CompletionKind::Variable
            && item.detail.as_deref() == Some("local")),
        "expected outer local completion, got: {:?}",
        outer_items
    );
    assert!(
        outer_items.iter().all(|item| item.label != "inner"),
        "did not expect inner block local outside its scope, got: {:?}",
        outer_items
    );
}

#[test]
fn test_completion_prioritizes_shadowed_local_name() {
    let tx = spawn_service();

    let source = r#"module app.main;

let value = 0;

flow main(value: i32) -> i32 {
    let outer = value;
    if true {
        let value = outer;
        return value$CURSOR$;
    }
    return 0;
}
"#;

    let items = completion_at_marker(&tx, "file:///completion_shadowing.es", source, "$CURSOR$");
    let value_items = items
        .iter()
        .filter(|item| item.label == "value" && item.kind == CompletionKind::Variable)
        .collect::<Vec<_>>();

    assert_eq!(
        value_items.len(),
        1,
        "expected one visible value completion, got: {:?}",
        items
    );
    assert_eq!(
        value_items[0].detail.as_deref(),
        Some("local"),
        "expected shadowing local to win, got: {:?}",
        items
    );
    assert!(
        items.iter().any(|item| item.label == "outer"
            && item.kind == CompletionKind::Variable
            && item.detail.as_deref() == Some("local")),
        "expected outer local to remain visible, got: {:?}",
        items
    );
}

#[test]
fn test_completion_does_not_leak_unimported_or_private_symbols() {
    let tx = spawn_service();
    let root = std::env::temp_dir().join("etas_lsp_completion_visibility_leak");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    std::fs::write(
        src_dir.join("utils.es"),
        r#"module utils;

public flow helper(x: i32) -> i32 {
    return x;
}

flow hidden(x: i32) -> i32 {
    return x;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

flow main() -> i32 {
    return $CURSOR$0;
}
"#;

    let items = project_completion_at_marker(&tx, &root, &app_path, app_source, "$CURSOR$");

    assert!(
        items.iter().all(|item| item.label != "helper"),
        "unimported public helper should not leak into expression completion: {:?}",
        items
    );
    assert!(
        items.iter().all(|item| item.label != "hidden"),
        "private helper should not leak into expression completion: {:?}",
        items
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_completion_includes_explicit_imported_symbol() {
    let tx = spawn_service();
    let root = std::env::temp_dir().join("etas_lsp_completion_explicit_import");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    std::fs::write(
        src_dir.join("utils.es"),
        r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.add;

flow main() -> i32 {
    return $CURSOR$0;
}
"#;

    let items = project_completion_at_marker(&tx, &root, &app_path, app_source, "$CURSOR$");

    assert!(
        items
            .iter()
            .any(|item| item.label == "add" && item.kind == CompletionKind::Flow),
        "expected imported flow completion, got: {:?}",
        items
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_completion_includes_explicit_imported_module() {
    let tx = spawn_service();
    let root = std::env::temp_dir().join("etas_lsp_completion_explicit_module_import");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    std::fs::write(
        src_dir.join("utils.es"),
        r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils;

flow main() -> i32 {
    return $CURSOR$0;
}
"#;

    let items = project_completion_at_marker(&tx, &root, &app_path, app_source, "$CURSOR$");

    assert!(
        items
            .iter()
            .any(|item| item.label == "utils" && item.kind == CompletionKind::Module),
        "expected imported module completion, got: {:?}",
        items
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_completion_wildcard_import_public_only() {
    let tx = spawn_service();
    let root = std::env::temp_dir().join("etas_lsp_completion_wildcard_public_only");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    std::fs::write(
        src_dir.join("utils.es"),
        r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}

flow private_add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.*;

flow main() -> i32 {
    return $CURSOR$0;
}
"#;

    let items = project_completion_at_marker(&tx, &root, &app_path, app_source, "$CURSOR$");

    assert!(
        items
            .iter()
            .any(|item| item.label == "add" && item.kind == CompletionKind::Flow),
        "expected public wildcard import completion, got: {:?}",
        items
    );
    assert!(
        items.iter().all(|item| item.label != "private_add"),
        "private wildcard export should not be visible, got: {:?}",
        items
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_completion_omits_ambiguous_wildcard_import() {
    let tx = spawn_service();
    let root = std::env::temp_dir().join("etas_lsp_completion_wildcard_ambiguous");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    std::fs::write(
        src_dir.join("a.es"),
        r#"module a;

public flow shared() -> i32 {
    return 1;
}
"#,
    )
    .unwrap();
    std::fs::write(
        src_dir.join("b.es"),
        r#"module b;

public flow shared() -> i32 {
    return 2;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import a.*;
import b.*;

flow main() -> i32 {
    return $CURSOR$0;
}
"#;

    let items = project_completion_at_marker(&tx, &root, &app_path, app_source, "$CURSOR$");

    assert!(
        items.iter().all(|item| item.label != "shared"),
        "ambiguous wildcard import should not be completed, got: {:?}",
        items
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_completion_local_shadows_imported_symbol() {
    let tx = spawn_service();
    let root = std::env::temp_dir().join("etas_lsp_completion_local_shadows_import");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    std::fs::write(
        src_dir.join("utils.es"),
        r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.add;

flow main() -> i32 {
    let add = 1;
    return $CURSOR$add;
}
"#;

    let items = project_completion_at_marker(&tx, &root, &app_path, app_source, "$CURSOR$");
    let add_items = items
        .iter()
        .filter(|item| item.label == "add")
        .collect::<Vec<_>>();

    assert_eq!(
        add_items.len(),
        1,
        "expected local shadow to produce one add completion, got: {:?}",
        items
    );
    assert_eq!(
        add_items[0].kind,
        CompletionKind::Variable,
        "expected local add to win over imported flow, got: {:?}",
        items
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_completion_current_module_private_symbol_visible() {
    let tx = spawn_service();

    let source = r#"module app;

flow private_helper() -> i32 {
    return 1;
}

flow main() -> i32 {
    return $CURSOR$0;
}
"#;

    let items = completion_at_marker(
        &tx,
        "file:///completion_current_module_private.es",
        source,
        "$CURSOR$",
    );

    assert!(
        items
            .iter()
            .any(|item| item.label == "private_helper" && item.kind == CompletionKind::Flow),
        "current module private helper should be visible, got: {:?}",
        items
    );
}

#[test]
fn test_completion_does_not_fail_on_parse_error() {
    let tx = spawn_service();
    let uri = "file:///broken_completion.es";
    open_and_check(&tx, uri, "module test\nflow");

    let items = completion(&tx, uri, 1, 4);

    assert!(
        items
            .iter()
            .any(|item| item.kind == CompletionKind::Keyword),
        "expected keyword fallback, got: {:?}",
        items
    );
}

#[test]
fn test_close_project_file_restores_disk_text() {
    let mut service = LanguageService::new();

    let root = std::env::temp_dir().join("etas_lsp_test_close_restores_disk_text");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

flow main() -> i32 {
    return 0;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let utils_path = src_dir.join("utils.es");
    let utils_source = r#"module utils;

flow helper() -> i32 {
    return 1;
}
"#;
    std::fs::write(&utils_path, utils_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let utils_uri = path_to_uri(&utils_path.canonicalize().unwrap());
    service.set_workspace_root(root.clone());
    service.open_document(app_uri.clone(), app_source.to_string(), 1);

    let dirty_source = r#"module app;

flow main() -> i32 {
    return 0;
}

flow main() -> i32 {
    return 1;
}
"#;
    service.change_document(
        app_uri.clone(),
        vec![LspTextEdit {
            range: None,
            text: dirty_source.to_string(),
        }],
        2,
    );
    service.close_document(app_uri.clone());
    service.open_document(utils_uri, utils_source.to_string(), 1);

    let app_errors: Vec<_> = service
        .diagnostics
        .get(&app_uri)
        .into_iter()
        .flatten()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        !app_errors.iter().any(|m| m.contains("duplicate")),
        "closing dirty project file should restore disk text, got: {:?}",
        app_errors
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_go_to_definition_type() {
    let tx = spawn_service();

    let source = r#"module test;

type UserId = string;

flow main() -> i32 {
    return 0;
}
"#;

    let uri = "file:///test.es";
    open_and_check(&tx, uri, source);

    let locs = goto_def(&tx, uri, 2, 8);
    assert!(
        !locs.is_empty(),
        "should find definition for type name on declaration line"
    );
    let loc = &locs[0];
    assert_eq!(loc.uri, uri);
    assert!(
        loc.range.start.0 >= 14 && loc.range.end.0 <= 36,
        "definition should be the type line, got {:?}",
        loc.range
    );
}

#[test]
fn test_go_to_definition_flow_call() {
    let tx = spawn_service();

    let source = r#"module test;

flow double(x: i32) -> i32 {
    return x + x;
}

flow main() -> i32 {
    let n = double(5);
    return n;
}
"#;

    let uri = "file:///test.es";
    open_and_check(&tx, uri, source);

    let locs = goto_def(&tx, uri, 7, 14);
    assert!(
        !locs.is_empty(),
        r#"should find definition for "double" call"#
    );
    let loc = &locs[0];
    assert_eq!(loc.uri, uri);
    assert!(
        loc.range.start.0 <= 19 && loc.range.end.0 >= 25,
        "definition should be flow declaration, got start={} end={}",
        loc.range.start.0,
        loc.range.end.0
    );
}

#[test]
fn test_go_to_definition_no_hit() {
    let tx = spawn_service();

    let source = r#"module test;

flow main() -> i32 {
    return 0;
}
"#;

    let uri = "file:///test.es";
    open_and_check(&tx, uri, source);

    let locs = goto_def(&tx, uri, 3, 13);
    assert!(locs.is_empty(), "no definition expected for literal");
}

#[test]
fn test_go_to_definition_type_reference() {
    let tx = spawn_service();

    let source = r#"module test;

alias Score = i32;

flow clamp(score: Score) -> Score {
    return score;
}

flow main() -> i32 {
    let s: Score = 8;
    return s;
}
"#;

    let uri = "file:///test.es";
    let diags = open_and_check(&tx, uri, source);
    assert!(diags.is_empty(), "source should be clean, got: {:?}", diags);

    let locs = goto_def(&tx, uri, 9, 13);
    assert!(
        !locs.is_empty(),
        "should find definition for type reference Score in let annotation, got: {:?}",
        locs
    );
}

#[test]
fn test_go_to_definition_retains_snapshot_on_syntax_error() {
    let tx = spawn_service();

    let valid_source = r#"module test;

flow double(x: i32) -> i32 {
    return x + x;
}

flow main() -> i32 {
    let n = double(5);
    return n;
}
"#;

    let uri = "file:///test.es";

    let diags1 = open_and_check(&tx, uri, valid_source);
    assert!(
        diags1.is_empty(),
        "valid source should have no errors: {:?}",
        diags1
    );

    let locs1 = goto_def(&tx, uri, 7, 14);
    assert!(!locs1.is_empty(), "goto should work on valid code");

    let broken_source = r#"module test

flow double(x: i32) -> i32 {
    return x + x;
}

flow main() -> i32 {
    let n = double(5);
    return n;
}
"#;

    let _ = tx.send(Command::ChangeDocument {
        uri: uri.to_string(),
        content_changes: vec![LspTextEdit {
            range: None,
            text: broken_source.to_string(),
        }],
        version: 2,
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let locs2 = goto_def(&tx, uri, 7, 14);
    assert!(
        !locs2.is_empty(),
        "goto should still work on broken code using retained snapshot"
    );

    let loc = &locs2[0];
    assert!(
        loc.range.start.0 <= 19 && loc.range.end.0 >= 25,
        "definition should still point to flow declaration, got start={} end={}",
        loc.range.start.0,
        loc.range.end.0
    );
}

#[test]
fn test_multifile_no_duplicate_or_ambiguous() {
    let tx = spawn_service();

    let root = std::env::temp_dir().join("etas_lsp_test3");
    let src_dir = root.join("src");
    let _ = std::fs::create_dir_all(&src_dir);

    // utils.es — helper module, no main
    std::fs::write(
        src_dir.join("utils.es"),
        r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}

flow multiply(a: i32, b: i32) -> i32 {
    return a * b;
}
"#,
    )
    .unwrap();

    // math.es — another helper, no main
    std::fs::write(
        src_dir.join("math.es"),
        r#"module math;

flow square(x: i32) -> i32 {
    return x * x;
}
"#,
    )
    .unwrap();

    // app.es — entry module with main
    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.add;
import utils.multiply;

flow main() -> i32 {
    let x = add(2, 3);
    let y = multiply(x, 4);
    return y;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let uri = format!("file://{}", app_path.display());
    let _ = tx.send(Command::SetWorkspaceRoot(root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &uri, app_source);

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GetDiagnostics {
        respond: respond_tx,
    });
    let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
    let app_diags = diags.get(&uri).cloned().unwrap_or_default();

    let error_messages: Vec<_> = app_diags
        .iter()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| &d.message)
        .collect();

    // Should have no "ambiguous" or "duplicate" errors
    assert!(
        !error_messages.iter().any(|m| m.contains("ambiguous")),
        "no ambiguous main, got: {:?}",
        error_messages
    );
    assert!(
        !error_messages.iter().any(|m| m.contains("duplicate")),
        "no duplicate symbols, got: {:?}",
        error_messages
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_goto_qualified_import_item_segment_jumps_to_item() {
    let tx = spawn_service();

    let root = std::env::temp_dir().join("etas_lsp_test_import_item_segment_goto");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    let utils_path = src_dir.join("utils.es");
    let utils_source = r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#;
    std::fs::write(&utils_path, utils_source).unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.add;

flow main() -> i32 {
    return 0;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let utils_uri = path_to_uri(&utils_path.canonicalize().unwrap());
    let _ = tx.send(Command::SetWorkspaceRoot(root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &app_uri, app_source);

    let locs = goto_def(&tx, &app_uri, 2, 14);
    assert!(
        locs.iter().any(|loc| {
            loc.uri == utils_uri && loc.range.start.0 as usize > "module utils;\n".len()
        }),
        "goto on import item segment should jump to flow add, got: {:?}",
        locs
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_goto_qualified_module_item_call_jumps_to_item() {
    let tx = spawn_service();

    let root = std::env::temp_dir().join("etas_lsp_test_qualified_module_item_goto");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    let utils_path = src_dir.join("utils.es");
    let utils_source = r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#;
    std::fs::write(&utils_path, utils_source).unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils;

flow main() -> i32 {
    return 0;
}
"#;
    let app_source_with_qualified_call = r#"module app;

import utils;

flow main() -> i32 {
    return utils.add(2, 3);
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let utils_uri = path_to_uri(&utils_path.canonicalize().unwrap());
    let _ = tx.send(Command::SetWorkspaceRoot(root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &app_uri, app_source);
    let _ = tx.send(Command::ChangeDocument {
        uri: app_uri.clone(),
        content_changes: vec![LspTextEdit {
            range: None,
            text: app_source_with_qualified_call.to_owned(),
        }],
        version: 2,
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let locs = goto_def(&tx, &app_uri, 5, 18);
    assert!(
        locs.iter().any(|loc| {
            loc.uri == utils_uri && loc.range.start.0 as usize > "module utils;\n".len()
        }),
        "goto on qualified call item segment should jump to flow add, got: {:?}",
        locs
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_goto_wildcard_import_call_jumps_to_item() {
    let tx = spawn_service();

    let root = std::env::temp_dir().join("etas_lsp_test_wildcard_import_goto");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    let utils_path = src_dir.join("utils.es");
    let utils_source = r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#;
    std::fs::write(&utils_path, utils_source).unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.*;

flow main() -> i32 {
    return add(2, 3);
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let utils_uri = path_to_uri(&utils_path.canonicalize().unwrap());
    let _ = tx.send(Command::SetWorkspaceRoot(root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &app_uri, app_source);

    let locs = goto_def(&tx, &app_uri, 5, 13);
    assert!(
        locs.iter().any(|loc| {
            loc.uri == utils_uri && loc.range.start.0 as usize > "module utils;\n".len()
        }),
        "goto on wildcard-imported call should jump to flow add, got: {:?}",
        locs
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_nested_multifile_infers_entry_module() {
    let tx = spawn_service();

    let outer_root = std::env::temp_dir().join("etas_lsp_test_nested_outer");
    let root = outer_root.join("tmp").join("examples").join("multifile");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&outer_root);
    let _ = std::fs::create_dir_all(&src_dir);

    let utils_path = src_dir.join("utils.es");
    std::fs::write(
        &utils_path,
        r#"module utils;

flow add(a: i32, b: i32) -> i32 {
    return a + b;
}

flow multiply(a: i32, b: i32) -> i32 {
    return a * b;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.add;

flow main() -> i32 {
    let x = add(2, 3);
    let y = utils.multiply(x, 4);
    return y;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let _ = tx.send(Command::SetWorkspaceRoot(outer_root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &app_uri, app_source);

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GetDiagnostics {
        respond: respond_tx,
    });
    let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
    let app_diags = diags.get(&app_uri).cloned().unwrap_or_default();
    let error_messages: Vec<_> = app_diags
        .iter()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| &d.message)
        .collect();

    assert!(
        !error_messages.iter().any(|m| m.contains("ambiguous")),
        "no ambiguous main, got: {:?}",
        error_messages
    );
    assert!(
        !error_messages
            .iter()
            .any(|m| m.contains("missing imported module")),
        "imports should resolve, got: {:?}",
        error_messages
    );

    let _ = std::fs::remove_dir_all(&outer_root);
}

#[test]
fn test_nested_multifile_overrides_unrelated_workspace_src() {
    let tx = spawn_service();

    let outer_root = std::env::temp_dir().join("etas_lsp_test_nested_with_outer_src");
    let outer_src = outer_root.join("src");
    let root = outer_root.join("tmp").join("examples").join("multifile");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&outer_root);
    let _ = std::fs::create_dir_all(&outer_src);
    let _ = std::fs::create_dir_all(&src_dir);

    std::fs::write(
        outer_src.join("unrelated.es"),
        r#"module unrelated;

flow main() -> i32 {
    return 0;
}
"#,
    )
    .unwrap();
    std::fs::write(
        src_dir.join("utils.es"),
        r#"module utils;

flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.add;

flow main() -> i32 {
    let x = add(2, 3);
    return x;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let _ = tx.send(Command::SetWorkspaceRoot(outer_root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &app_uri, app_source);

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GetDiagnostics {
        respond: respond_tx,
    });
    let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
    let app_diags = diags.get(&app_uri).cloned().unwrap_or_default();
    let error_messages: Vec<_> = app_diags
        .iter()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| &d.message)
        .collect();

    assert!(
        !error_messages
            .iter()
            .any(|m| m.contains("missing imported module")),
        "imports should resolve, got: {:?}",
        error_messages
    );

    let _ = std::fs::remove_dir_all(&outer_root);
}

#[test]
fn test_recursive_project_discovery_resolves_nested_modules() {
    let tx = spawn_service();

    let root = std::env::temp_dir().join("etas_lsp_test_recursive_project_discovery");
    let app_dir = root.join("src").join("app");
    let checkout_dir = app_dir.join("checkout");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&checkout_dir);

    let cart_path = checkout_dir.join("cart.es");
    std::fs::write(
        &cart_path,
        r#"module app.checkout.cart;

public type Cart = {
    total: i32,
}

public flow sample_cart() -> Cart ![] {
    return Cart { total = 10 };
}
"#,
    )
    .unwrap();

    let main_path = app_dir.join("main.es");
    let main_source = r#"module app.main;

import app.checkout.cart.{sample_cart};

flow main() -> i32 ![] {
    let cart = sample_cart();
    return cart.total;
}
"#;
    std::fs::write(&main_path, main_source).unwrap();

    let main_uri = format!("file://{}", main_path.display());
    let _ = tx.send(Command::SetWorkspaceRoot(root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &main_uri, main_source);

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GetDiagnostics {
        respond: respond_tx,
    });
    let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
    let main_diags = diags.get(&main_uri).cloned().unwrap_or_default();
    let error_messages: Vec<_> = main_diags
        .iter()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| &d.message)
        .collect();

    assert!(
        !error_messages
            .iter()
            .any(|m| m.contains("missing imported module `app.checkout.cart`")),
        "nested import should resolve, got: {:?}",
        error_messages
    );
    assert!(
        !error_messages.iter().any(|m| m.contains("entry flow")),
        "entry module should be app.main, got: {:?}",
        error_messages
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_opening_same_module_name_in_different_packages_does_not_duplicate() {
    let tx = spawn_service();

    let outer_root = std::env::temp_dir().join("etas_lsp_test_same_module_different_packages");
    let tmp_root = outer_root.join("tmp").join("examples").join("multifile");
    let example_root = outer_root.join("examples").join("multifile");
    let tmp_src = tmp_root.join("src");
    let example_src = example_root.join("src");
    let _ = std::fs::remove_dir_all(&outer_root);
    let _ = std::fs::create_dir_all(&tmp_src);
    let _ = std::fs::create_dir_all(&example_src);

    let app_source = r#"module app;

flow main() -> i32 {
    return 0;
}
"#;
    let utils_source = r#"module utils;

public flow add(a: i32, b: i32) -> i32 {
    return a + b;
}
"#;

    let tmp_app_path = tmp_src.join("app.es");
    let example_app_path = example_src.join("app.es");
    std::fs::write(&tmp_app_path, app_source).unwrap();
    std::fs::write(tmp_src.join("utils.es"), utils_source).unwrap();
    std::fs::write(&example_app_path, app_source).unwrap();
    std::fs::write(example_src.join("utils.es"), utils_source).unwrap();

    let tmp_uri = format!("file://{}", tmp_app_path.display());
    let example_uri = format!("file://{}", example_app_path.display());
    let _ = tx.send(Command::SetWorkspaceRoot(outer_root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &tmp_uri, app_source);
    open_file(&tx, &example_uri, app_source);

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GetDiagnostics {
        respond: respond_tx,
    });
    let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
    let example_diags = diags.get(&example_uri).cloned().unwrap_or_default();
    let error_messages: Vec<_> = example_diags
        .iter()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| &d.message)
        .collect();

    assert!(
        !error_messages.iter().any(|m| m.contains("duplicate")),
        "opening same module in another package should not produce duplicate errors: {:?}",
        error_messages
    );

    for uri in [&tmp_uri, &example_uri] {
        let _ = tx.send(Command::ChangeDocument {
            uri: uri.clone(),
            content_changes: vec![LspTextEdit {
                range: None,
                text: app_source.to_string(),
            }],
            version: 2,
        });
        std::thread::sleep(std::time::Duration::from_millis(50));

        let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(Command::GetDiagnostics {
            respond: respond_tx,
        });
        let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
        let current_diags = diags.get(uri).cloned().unwrap_or_default();
        let error_messages: Vec<_> = current_diags
            .iter()
            .filter(|d| d.severity == IntelSeverity::Error)
            .map(|d| &d.message)
            .collect();
        assert!(
            !error_messages.iter().any(|m| m.contains("duplicate")),
            "switching back to {uri} should not produce duplicate errors: {:?}",
            error_messages
        );
    }

    let _ = std::fs::remove_dir_all(&outer_root);
}

#[test]
fn test_rename_open_project_file_does_not_add_duplicate_source() {
    let mut service = LanguageService::new();

    let root = std::env::temp_dir().join("etas_lsp_test_rename_open_project_file");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

flow main() -> i32 {
    return 0;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let math_path = src_dir.join("math.es");
    let math_source = r#"module math;

flow square(x: i32) -> i32 {
    return x * x;
}
"#;
    std::fs::write(&math_path, math_source).unwrap();

    service.set_workspace_root(root.clone());
    let app_uri = format!("file://{}", app_path.display());
    service.open_document(app_uri.clone(), app_source.to_string(), 1);

    let app1_path = src_dir.join("app1.es");
    let app1_source = r#"module app1;

flow main() -> i32 {
    return 0;
}
"#;
    std::fs::rename(&app_path, &app1_path).unwrap();
    std::fs::write(&app1_path, app1_source).unwrap();

    service.remove_source_file(app_path.to_string_lossy().to_string());
    let app1_uri = format!("file://{}", app1_path.display());
    service.open_document(app1_uri.clone(), app1_source.to_string(), 1);
    service.add_source_file(app1_path.to_string_lossy().to_string());
    service.change_document(
        app1_uri.clone(),
        vec![LspTextEdit {
            range: None,
            text: app1_source.to_string(),
        }],
        2,
    );

    let errors: Vec<_> = service
        .diagnostics
        .get(&app1_uri)
        .into_iter()
        .flatten()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        !errors.iter().any(|m| m.contains("duplicate")),
        "renamed open file should not be added twice, got: {:?}",
        errors
    );

    let all_errors: Vec<_> = service
        .diagnostics
        .values()
        .flatten()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        !all_errors
            .iter()
            .any(|m| m.contains("entry module `app` was not found")),
        "renaming the entry file should recompute entry module, got: {:?}",
        all_errors
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_goto_after_deleting_and_restoring_qualified_call() {
    let tx = spawn_service();

    let root = std::env::temp_dir().join("etas_lsp_test_restore_qualified_call");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    let utils_path = src_dir.join("utils.es");
    std::fs::write(
        &utils_path,
        r#"module utils;

public type Score = i32;
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.Score;

flow main() -> i32 {
    let x: Score = 3;
    return x;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let utils_uri = path_to_uri(&utils_path.canonicalize().unwrap());
    let _ = tx.send(Command::SetWorkspaceRoot(root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &app_uri, app_source);

    let _ = tx.send(Command::ChangeDocument {
        uri: app_uri.clone(),
        content_changes: vec![LspTextEdit {
            range: Some(LspRange {
                start: LspPosition {
                    line: 5,
                    character: 11,
                },
                end: LspPosition {
                    line: 5,
                    character: 16,
                },
            }),
            text: String::new(),
        }],
        version: 2,
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let _ = tx.send(Command::ChangeDocument {
        uri: app_uri.clone(),
        content_changes: vec![LspTextEdit {
            range: Some(LspRange {
                start: LspPosition {
                    line: 5,
                    character: 11,
                },
                end: LspPosition {
                    line: 5,
                    character: 11,
                },
            }),
            text: "Score".to_owned(),
        }],
        version: 3,
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let (respond_tx, respond_rx) = tokio::sync::oneshot::channel();
    let _ = tx.send(Command::GetDiagnostics {
        respond: respond_tx,
    });
    let (diags, _) = respond_rx.blocking_recv().unwrap_or_default();
    let app_diags = diags.get(&app_uri).cloned().unwrap_or_default();
    let error_messages: Vec<_> = app_diags
        .iter()
        .filter(|d| d.severity == IntelSeverity::Error)
        .map(|d| &d.message)
        .collect();
    assert!(
        error_messages.is_empty(),
        "restored document should have no errors, got: {:?}",
        error_messages
    );

    let locs = goto_def(&tx, &app_uri, 5, 13);
    assert!(
        locs.iter().any(|loc| loc.uri == utils_uri),
        "goto should resolve restored qualified call to utils.es, got: {:?}",
        locs
    );
    assert!(
        locs.iter().all(|loc| !loc.uri.contains("unknown_source")),
        "goto should not return unknown source URI, got: {:?}",
        locs
    );
    assert!(
        locs.iter().any(|loc| !loc.source_text.is_empty()),
        "target location should include source text for range conversion"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_goto_to_closed_project_file_keeps_uri() {
    let tx = spawn_service();

    let root = std::env::temp_dir().join("etas_lsp_test_closed_project_file_goto");
    let src_dir = root.join("src");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::create_dir_all(&src_dir);

    let utils_path = src_dir.join("utils.es");
    let utils_source = r#"module utils;

public type Score = i32;
"#;
    std::fs::write(&utils_path, utils_source).unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import utils.Score;

flow main() -> i32 {
    let x: Score = 3;
    return x;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let utils_uri = path_to_uri(&utils_path.canonicalize().unwrap());
    let _ = tx.send(Command::SetWorkspaceRoot(root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));

    open_file(&tx, &app_uri, app_source);
    open_file(&tx, &utils_uri, utils_source);
    let _ = tx.send(Command::CloseDocument {
        uri: utils_uri.clone(),
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let locs = goto_def(&tx, &app_uri, 5, 13);
    assert!(
        locs.iter().any(|loc| loc.uri == utils_uri),
        "goto should resolve closed project file URI, got: {:?}",
        locs
    );
    assert!(
        locs.iter().all(|loc| !loc.uri.contains("unknown_source")),
        "goto should not return unknown source URI, got: {:?}",
        locs
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_lsp_refreshes_external_package_effect_facts_after_prepare() {
    let root = std::env::temp_dir().join(format!(
        "etas_lsp_test_external_effect_facts_{}",
        std::process::id()
    ));
    let src_dir = root.join("src");
    let dep_src_dir = root.join("deps/remote-api/src/remote");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&dep_src_dir).unwrap();

    std::fs::write(
        root.join("deps/remote-api/etas.toml"),
        r#"[package]
name = "remote-api"
version = "0.1.0"
edition = "2026"

[source]
root = "src"
"#,
    )
    .unwrap();
    std::fs::write(
        dep_src_dir.join("api.es"),
        r#"module remote.api;

public effect Remote {
    action request(url: string) -> string;
}

public flow fetch(url: string) -> string ![Remote.request] {
    return perform Remote.request(url);
}
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("etas.toml"),
        r#"[package]
name = "lsp-external-effect-facts"
version = "0.1.0"
edition = "2026"

[source]
root = "src"

[dependencies]
remote_api = { package = "remote-api", version = "0.1.0", import = "remote.api", path = "deps/remote-api" }

[[bin]]
name = "lsp-external-effect-facts"
module = "app"
flow = "main"
"#,
    )
    .unwrap();

    let app_path = src_dir.join("app.es");
    let app_source = r#"module app;

import remote.api.{Remote, fetch};

flow main() -> i32 ![Remote.request] {
    let body = fetch("https://example.com/status");
    return 0;
}
"#;
    std::fs::write(&app_path, app_source).unwrap();

    let dep_root = root.join("deps/remote-api");
    etas_package::materialize_package(etas_package::MaterializePackageOptions {
        package_root: dep_root.clone(),
        source_config: None,
    })
    .unwrap();
    etas_package::lock_package(etas_package::LockPackageOptions {
        package_root: dep_root.clone(),
    })
    .unwrap();
    etas_driver::build_package_metadata(
        &etas_driver::DriverOptions::default(),
        etas_driver::PackageMetadataRequest {
            package_root: dep_root,
            bin: None,
        },
    )
    .unwrap();
    etas_driver::update_package(
        &etas_driver::DriverOptions::default(),
        etas_driver::PackageUpdateRequest {
            package_root: root.clone(),
        },
    )
    .unwrap();
    let prepared = etas_package::prepare_package(etas_package::PreparePackageOptions {
        package_root: root.clone(),
        selected_bin: None,
    })
    .unwrap();
    let environment =
        etas_driver::package_env::frontend_environment(prepared.environment.clone()).unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let mut service = LanguageService::new();
    service.set_workspace_root(root.clone());
    service.package_state = Some(PreparedPackageState {
        root: prepared.package_root,
        source_root: prepared.source_root,
        selected_bin: prepared.selected_bin,
        environment,
        package_environment: prepared.environment,
    });
    service.open_document(app_uri.clone(), app_source.to_string(), 1);

    let diagnostics = service
        .diagnostics
        .get(&app_uri)
        .cloned()
        .unwrap_or_default();
    let messages: Vec<_> = diagnostics
        .iter()
        .map(|diag| diag.message.as_str())
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        !messages
            .iter()
            .any(|message| message.contains("source import target requires checked effect facts")),
        "package prepare should refresh external effect facts diagnostics, got: {:?}",
        messages
    );
    assert!(
        messages.is_empty(),
        "expected clean diagnostics, got: {:?}",
        messages
    );
}

#[test]
fn test_nested_example_package_activates_path_dependency_from_outer_workspace() {
    let workspace_root = std::env::temp_dir().join(format!(
        "etas_lsp_test_nested_example_path_dep_{}",
        std::process::id()
    ));
    let example_root = workspace_root.join("examples/edk-http-fetch");
    let example_src = example_root.join("src");
    let dep_root = workspace_root.join("packages/edk-http");
    let dep_src = dep_root.join("src/edk");
    let _ = std::fs::remove_dir_all(&workspace_root);
    std::fs::create_dir_all(&example_src).unwrap();
    std::fs::create_dir_all(&dep_src).unwrap();

    std::fs::write(
        workspace_root.join("etas.toml"),
        r#"[workspace]
members = [
  "packages/edk-http",
]
"#,
    )
    .unwrap();
    std::fs::write(
        dep_root.join("etas.toml"),
        r#"[package]
name = "edk-http"
version = "0.1.0"
edition = "2026"

[source]
root = "src"
"#,
    )
    .unwrap();
    let dep_api_path = dep_src.join("http.es");
    std::fs::write(
        &dep_api_path,
        r#"module edk.http;

public flow get(url: string) -> i32 ![] {
    return 200;
}
"#,
    )
    .unwrap();
    std::fs::write(
        example_root.join("etas.toml"),
        r#"[package]
name = "edk-http-fetch-example"
version = "0.1.0"
edition = "2026"

[source]
root = "src"

[dependencies]
edk_http = { path = "../../packages/edk-http", import = "edk.http" }

[[bin]]
name = "edk-http-fetch-example"
module = "main"
flow = "main"
"#,
    )
    .unwrap();

    let marked_source = r#"module main;

import edk.http.{get};

flow main() -> i32 ![] {
    return get/*MARK*/("https://example.com");
}
"#;
    let (app_source, line, character) =
        source_without_marker_and_position(marked_source, "/*MARK*/");
    let app_path = example_src.join("main.es");
    std::fs::write(&app_path, &app_source).unwrap();

    etas_package::materialize_package(etas_package::MaterializePackageOptions {
        package_root: dep_root.clone(),
        source_config: None,
    })
    .unwrap();
    etas_package::lock_package(etas_package::LockPackageOptions {
        package_root: dep_root.clone(),
    })
    .unwrap();
    etas_driver::build_package_metadata(
        &etas_driver::DriverOptions::default(),
        etas_driver::PackageMetadataRequest {
            package_root: dep_root,
            bin: None,
        },
    )
    .unwrap();
    etas_driver::update_package(
        &etas_driver::DriverOptions::default(),
        etas_driver::PackageUpdateRequest {
            package_root: example_root.clone(),
        },
    )
    .unwrap();

    let app_uri = format!("file://{}", app_path.display());
    let tx = spawn_service();
    let _ = tx.send(Command::SetWorkspaceRoot(workspace_root.clone()));
    std::thread::sleep(std::time::Duration::from_millis(10));
    open_file(&tx, &app_uri, &app_source);

    let locations = goto_def(&tx, &app_uri, line, character);
    let _ = std::fs::remove_dir_all(&workspace_root);
    assert!(
        locations.iter().any(|location| location
            .uri
            .ends_with("/.etas/vendor/edk/http/src/edk/http.es")),
        "nested example should activate its own package and resolve edk.http dependency, got: {:?}",
        locations
    );
}
