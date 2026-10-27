use super::*;
use crate::IntelSeverity;

fn checked(source: &str) -> (LanguageService, String) {
    let uri = format!("file:///semantic_tokens_{}.es", std::process::id());
    let mut service = LanguageService::new();
    service.open_document(uri.clone(), source.to_owned(), 1);
    service.ensure_fresh_analysis();
    let diagnostics = service.get_diagnostics();
    assert!(
        diagnostics
            .values()
            .flatten()
            .all(|diag| diag.severity != IntelSeverity::Error),
        "{diagnostics:?}"
    );
    (service, uri)
}

fn assert_kind(service: &LanguageService, uri: &str, needle: &str, expected: &str) {
    let (tokens, text) = service.semantic_tokens(uri);
    assert_token_kind(&tokens, &text, needle, expected);
}

pub(super) fn assert_snapshot_kind(
    snapshot: &ProjectSemanticSnapshot,
    source: SourceId,
    text: &str,
    needle: &str,
    expected: &str,
) {
    let parsed = crate::syntax::parse(text);
    let tokens = crate::semantic_tokens::collect(snapshot, source, &parsed);
    let tokens =
        crate::syntax::merge_tokens(&parsed, crate::syntax::lexical_tokens(&parsed), tokens);
    assert_token_kind(&tokens, text, needle, expected);
}

fn assert_token_kind(tokens: &[IntelSemanticToken], text: &str, needle: &str, expected: &str) {
    let offsets = text
        .match_indices(needle)
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    assert!(!offsets.is_empty(), "missing {needle}");
    for offset in offsets {
        let token = tokens
            .iter()
            .find(|token| token.byte_start as usize == offset);
        let actual = token.map(|token| crate::SEMANTIC_TOKEN_TYPES[token.token_type as usize]);
        assert_eq!(
            actual,
            Some(expected),
            "wrong classification for {needle:?} at {offset}"
        );
    }
}

#[test]
fn standard_import_alias_calls_and_references_are_functions() {
    let (service, uri) = checked(
        r#"module tokens;
import std.collections.len as count;
import std.text.{trim as clean};
flow parse(args: Array<string>) -> usize {
    let f = count;
    let text = clean(" x ");
    if count(args) == 1 { return count(args); }
    return count(args);
}
flow main() -> i32 { return 0; }
"#,
    );
    assert_kind(&service, &uri, "count;", "function");
    assert_kind(&service, &uri, "count(args)", "function");
    assert_kind(&service, &uri, "clean};", "function");
    assert_kind(&service, &uri, "clean(\"", "function");
}

#[test]
fn std_types_modules_wildcards_and_prelude_are_not_all_functions() {
    let source = "module tokens;\nimport std.collections.Array as Seq;\nimport std.text as text_ops;\nimport std.text.*;\nflow parse(args: Seq<string>) -> usize { return len(\"x\"); }\nflow maybe() -> Option<i32> { return Some(1); }\nflow main() -> i32 { return 0; }\n";
    let (service, uri) = checked(source);
    assert_kind(&service, &uri, "Seq;", "type");
    assert_kind(&service, &uri, "Seq<string>", "type");
    assert_kind(&service, &uri, "text_ops;", "namespace");
    assert_kind(&service, &uri, "len(\"x\")", "function");
    assert_kind(&service, &uri, "Some(1)", "enumMember");
}

#[test]
fn local_shadowing_keeps_variables_and_parameters() {
    let source = "module tokens;\nimport std.collections.len as count;\nflow parse(args: Array<string>) -> usize {\n    let n = count(args);\n    if true { let count = n; let value = count; }\n    return count(args);\n}\nflow value(count: i32) -> i32 { return count; }\nflow main() -> i32 { return 0; }\n";
    let (service, uri) = checked(source);
    assert_kind(&service, &uri, "count(args)", "function");
    assert_kind(&service, &uri, "count = n", "variable");
    assert_kind(&service, &uri, "count; }\n    return", "variable");
    assert_kind(&service, &uri, "count: i32", "parameter");
    assert_kind(&service, &uri, "count; }\nflow main", "parameter");
}

#[test]
fn record_field_labels_and_generic_parameters_keep_their_kinds() {
    let (service, uri) = checked(
        "module tokens;\nalias Row<T> = { value: T, count: i32 };\nflow identity<T>(row: Row<T>) -> Row<T> { return row; }\nflow main() -> i32 { return 0; }\n",
    );
    assert_kind(&service, &uri, "value: T", "property");
    assert_kind(&service, &uri, "count: i32", "property");
    assert_kind(&service, &uri, "T, count", "typeParameter");
    assert_kind(&service, &uri, "T>)", "typeParameter");
}

#[test]
fn source_imports_types_and_modules_keep_target_kinds() {
    let root = std::env::temp_dir().join(format!("etas_semantic_imports_{}", std::process::id()));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("values.es"), "module values;\npublic alias Label = string;\npublic spec Region;\npublic flow square(x: i32) -> i32 { return x * x; }\n").unwrap();
    let source = "module main;\nimport values.{Label as Name, Region as Area, square as forwarded};\nimport values as library;\nflow show(name: Name) -> i32 { return forwarded(2); }\nflow main() -> i32 { return 0; }\n";
    let file = src.join("main.es");
    std::fs::write(&file, source).unwrap();
    let uri = path_to_uri(&file);
    let mut service = LanguageService::new();
    service.set_workspace_root(root.clone());
    service.open_document(uri.clone(), source.to_owned(), 1);
    service.ensure_fresh_analysis();
    assert!(
        service
            .get_diagnostics()
            .values()
            .flatten()
            .all(|diag| diag.severity != IntelSeverity::Error),
        "{:?}",
        service.get_diagnostics()
    );
    assert_kind(&service, &uri, "Name,", "type");
    assert_kind(&service, &uri, "Name)", "type");
    assert_kind(&service, &uri, "Area,", "interface");
    assert_kind(&service, &uri, "library;", "namespace");
    assert_kind(&service, &uri, "forwarded(2)", "function");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn action_declarations_and_perform_calls_are_functions() {
    let (service, uri) = checked(
        "module tokens;\neffect Store extends FileIO { action read(key: string) -> string; }\nflow read_value(key: string) -> string ![Store.read] { return perform Store.read(key); }\nflow main() -> i32 { return 0; }\n",
    );
    assert_kind(&service, &uri, "read(key", "function");
    assert_kind(&service, &uri, "Store extends", "interface");
}

#[test]
fn contextual_keyword_names_use_checked_name_spans() {
    let source = "module tokens;\nflow resume() -> i32 { return 0; }\nflow flow() -> i32 { return 0; }\nflow main() -> i32 { return 0; }\n";
    let (service, uri) = checked(source);
    assert_kind(&service, &uri, "resume()", "function");
    assert_kind(&service, &uri, "flow()", "function");
    assert_kind(&service, &uri, "flow resume", "keyword");
    assert_kind(&service, &uri, "return 0", "keyword");
    let hover = service
        .hover(
            &uri,
            LspPosition {
                line: 1,
                character: 5,
            },
        )
        .expect("a checked keyword-named function should have hover");
    assert!(
        hover.text.contains("flow resume() -> i32"),
        "{}",
        hover.text
    );
    assert!(
        service
            .hover(
                &uri,
                LspPosition {
                    line: 1,
                    character: 0
                }
            )
            .is_none(),
        "the declaration keyword itself is not the function name"
    );
}

#[test]
fn record_constructors_labels_and_values_have_distinct_tokens() {
    let (service, uri) = checked(
        r#"module tokens;
import std.text.split;
type Identity = { id: string, workspace: string, allowed: Array<string> };
type Envelope = { identity: Identity };
alias Row = { label: string };
flow make(id: string, workspace: string, allow: string) -> Envelope {
    let identity = Identity { id = id, workspace = workspace, allowed = split(allow, ",") };
    let row = Row { label = id };
    return Envelope { identity = identity };
}
flow main() -> i32 { return 0; }
"#,
    );
    assert_kind(&service, &uri, "Identity {", "type");
    assert_kind(&service, &uri, "Envelope {", "type");
    assert_kind(&service, &uri, "Row {", "type");
    assert_kind(&service, &uri, "id = id", "property");
    assert_kind(&service, &uri, "id, workspace", "parameter");
    assert_kind(&service, &uri, "workspace = workspace", "property");
    assert_kind(&service, &uri, "workspace, allowed", "parameter");
    assert_kind(&service, &uri, "identity = identity", "property");
    assert_kind(&service, &uri, "identity };", "variable");
    assert_kind(&service, &uri, "split(allow", "function");
    let hover = service
        .hover(&uri, source_position(&service, &uri, "Identity {"))
        .expect("record constructor should expose its checked type");
    assert!(hover.text.contains("type Identity"), "{}", hover.text);
}

#[test]
fn generic_and_anonymous_record_construction_uses_checked_hir() {
    let (service, uri) = checked(
        "module tokens;\ntype Box<T> = { value: T };\nflow boxed<T>(value: T) -> Box<T> { return Box<T> { value = value }; }\nflow main() -> i32 { let plain = 1; let row = { Identity = 1 }; let other = { plain }; return row.Identity; }\n",
    );
    assert_kind(&service, &uri, "Box<T> {", "type");
    assert_kind(&service, &uri, "T> {", "typeParameter");
    assert_kind(&service, &uri, "Identity = 1", "property");
    assert_kind(&service, &uri, "plain };", "variable");
}

#[test]
fn named_enum_payloads_remain_enum_members_not_record_types() {
    let (service, uri) = checked(
        "module tokens;\nenum Event { Data { id: string }; }\nflow make(id: string) -> Event { return Event.Data { id = id }; }\nflow main() -> i32 { return 0; }\n",
    );
    assert_kind(&service, &uri, "Event.Data", "type");
    assert_kind(&service, &uri, "Data {", "enumMember");
    assert_kind(&service, &uri, "id = id", "property");
}

fn source_position(service: &LanguageService, uri: &str, needle: &str) -> LspPosition {
    let text = &service.documents[uri].text;
    let before = &text[..text.find(needle).expect("test cursor must exist")];
    LspPosition {
        line: before.bytes().filter(|byte| *byte == b'\n').count() as u32,
        character: before.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
    }
}

#[test]
fn invalid_record_constructor_does_not_reuse_old_type_highlights() {
    let source = "module tokens;\ntype Identity = { id: string };\nflow main() -> i32 { let item = Identity { id = \"x\" }; return 0; }\n";
    let (mut service, uri) = checked(source);
    assert_kind(&service, &uri, "Identity {", "type");
    let broken = source.replace("= Identity {", "= Missing {");
    service.change_document(
        uri.clone(),
        vec![LspTextEdit {
            range: None,
            text: broken.clone(),
        }],
        2,
    );
    service.ensure_fresh_analysis();
    let (tokens, _) = service.semantic_tokens(&uri);
    let offset = broken.find("Missing {").unwrap();
    assert!(
        tokens
            .iter()
            .all(|token| token.byte_start as usize != offset)
    );
    assert!(
        service
            .hover(&uri, source_position(&service, &uri, "Missing {"))
            .is_none()
    );
}

#[test]
fn imported_record_constructor_alias_is_a_type() {
    imported_record_fixture(false);
}

#[test]
#[ignore = "compiler blocker: module-qualified nominal record constructor is unresolved; see docs/compiler-blockers.md"]
fn blocked_module_qualified_record_constructor() {
    imported_record_fixture(true);
}

fn imported_record_fixture(qualified: bool) {
    let root = std::env::temp_dir().join(format!(
        "etas_record_imports_{}_{}",
        std::process::id(),
        qualified
    ));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("models.es"),
        "module models;\npublic type Identity = { id: string };\n",
    )
    .unwrap();
    let source = if qualified {
        "module main;\nimport models.Identity as User;\nimport models;\nflow make(id: string) -> User { return User { id = id }; }\nflow qualified(id: string) -> User { return models.Identity { id = id }; }\nflow main() -> i32 { return 0; }\n"
    } else {
        "module main;\nimport models.Identity as User;\nflow make(id: string) -> User { return User { id = id }; }\nflow main() -> i32 { return 0; }\n"
    };
    let file = src.join("main.es");
    std::fs::write(&file, source).unwrap();
    let uri = path_to_uri(&file);
    let mut service = LanguageService::new();
    service.set_workspace_root(root.clone());
    service.open_document(uri.clone(), source.to_owned(), 1);
    service.ensure_fresh_analysis();
    let diagnostics = service.get_diagnostics().clone();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        diagnostics
            .values()
            .flatten()
            .all(|diag| diag.severity != IntelSeverity::Error),
        "{diagnostics:?}"
    );
    assert_kind(&service, &uri, "User {", "type");
    if qualified {
        assert_kind(&service, &uri, "models.Identity {", "namespace");
        assert_kind(&service, &uri, "Identity {", "type");
    }
}

#[test]
fn stale_invalid_edits_do_not_paint_unresolved_calls_as_functions() {
    let source = "module tokens;\nimport std.collections.len as count;\nflow main() -> i32 { let text = \"count\"; let n = count(\"x\"); return 0; }\n";
    let (mut service, uri) = checked(source);
    assert_kind(&service, &uri, "count(\"x\")", "function");
    service.change_document(
        uri.clone(),
        vec![LspTextEdit {
            range: None,
            text: source.replace("count(\"x\")", "missing(\"x\")"),
        }],
        2,
    );
    service.ensure_fresh_analysis();
    let (tokens, text) = service.semantic_tokens(&uri);
    let offset = text.find("missing(").unwrap();
    assert!(
        tokens
            .iter()
            .all(|token| token.byte_start as usize != offset),
        "an unresolved call must not become a guessed function"
    );
    service.change_document(
        uri.clone(),
        vec![LspTextEdit {
            range: None,
            text: source.to_owned(),
        }],
        3,
    );
    service.ensure_fresh_analysis();
    assert_kind(&service, &uri, "count(\"x\")", "function");
}

#[test]
#[ignore = "compiler blocker: imported std Array is not accepted as LengthInput; see docs/compiler-blockers.md"]
fn blocked_imported_std_array_satisfies_length_input() {
    let (service, uri) = checked(
        "module tokens;\nimport std.collections.Array as Seq;\nimport std.collections.len as count;\nflow size(args: Seq<string>) -> usize { return count(args); }\nflow main() -> i32 { return 0; }\n",
    );
    assert_kind(&service, &uri, "count(args)", "function");
}

#[test]
#[ignore = "compiler blocker: renamed source re-export lacks checked type/effect facts; see docs/compiler-blockers.md"]
fn blocked_renamed_source_reexport_keeps_callable_facts() {
    let root = std::env::temp_dir().join(format!("etas_blocked_reexport_{}", std::process::id()));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("values.es"),
        "module values;\npublic flow square(x: i32) -> i32 { return x * x; }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("bridge.es"),
        "module bridge;\npublic import values.square as forwarded;\n",
    )
    .unwrap();
    let source = "module main;\nimport bridge.*;\nflow main() -> i32 { return forwarded(2); }\n";
    let file = src.join("main.es");
    std::fs::write(&file, source).unwrap();
    let uri = path_to_uri(&file);
    let mut service = LanguageService::new();
    service.set_workspace_root(root.clone());
    service.open_document(uri.clone(), source.to_owned(), 1);
    service.ensure_fresh_analysis();
    let diagnostics = service.get_diagnostics().clone();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        diagnostics
            .values()
            .flatten()
            .all(|diag| diag.severity != IntelSeverity::Error),
        "{diagnostics:?}"
    );
    assert_kind(&service, &uri, "forwarded(2)", "function");
}
