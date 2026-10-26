use std::collections::{HashMap, HashSet};

use etas_core::{SourceId, TextSize};
use etas_frontend::{ImportTarget, ModuleId, ModulePartId, ModulePath, ProjectSemanticSnapshot};
use etas_hir::{
    HirModuleId, HirNodeRef, HirTreeView, ResolveResult, ScopeId, ScopeOwner, Symbol, SymbolDef,
    SymbolKind, SyntaxNodeKind, SyntaxNodeRef, Visibility,
};
use etas_types::{ItemSignature, Type, TypeId, TypeStore};

use crate::service::{DocumentState, offset_of_position};
use crate::types::completion::{CompletionItem, CompletionKind, CompletionParams};

pub(crate) struct CompletionRequest<'a> {
    pub(crate) documents: &'a HashMap<String, DocumentState>,
    pub(crate) snapshot: Option<&'a ProjectSemanticSnapshot>,
    pub(crate) params: CompletionParams,
}

pub(crate) fn complete(request: CompletionRequest<'_>) -> Vec<CompletionItem> {
    let mut candidates = Vec::new();

    let Some(doc) = request.documents.get(&request.params.uri) else {
        return Vec::new();
    };

    let context = completion_context(doc, request.params.position, request.snapshot);

    if let Some(snapshot) = request.snapshot {
        let offset = offset_of_position(&doc.text, request.params.position);
        if let Some(scope) = completion_scope_at(doc, offset, snapshot) {
            collect_visible_scope_candidates(
                &mut candidates,
                snapshot,
                scope,
                doc.source_id,
                offset,
                context,
            );
        }
    }

    if matches!(
        context,
        CompletionContext::Type | CompletionContext::Unknown
    ) {
        for primitive in PRIMITIVE_TYPE_COMPLETIONS {
            candidates.push(Candidate {
                item: CompletionItem {
                    label: (*primitive).to_string(),
                    kind: CompletionKind::Type,
                    detail: Some("primitive type".to_string()),
                    insert_text: None,
                },
                priority: CandidatePriority::PrimitiveType,
            });
        }
    }

    for keyword in completion_keywords(context) {
        candidates.push(Candidate {
            item: CompletionItem {
                label: (*keyword).to_string(),
                kind: CompletionKind::Keyword,
                detail: Some("keyword".to_string()),
                insert_text: None,
            },
            priority: CandidatePriority::Keyword,
        });
    }

    finalize_candidates(candidates)
}

#[derive(Clone, Debug)]
struct Candidate {
    item: CompletionItem,
    priority: CandidatePriority,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CandidatePriority {
    Local,
    Param,
    TypeParam,
    Import,
    CurrentModule,
    PrimitiveType,
    Keyword,
}

fn finalize_candidates(mut candidates: Vec<Candidate>) -> Vec<CompletionItem> {
    candidates.sort_by_key(|candidate| candidate.priority);

    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for candidate in candidates {
        if seen.insert(candidate.item.label.clone()) {
            items.push(candidate.item);
        }
    }
    items
}

fn completion_scope_at(
    doc: &DocumentState,
    offset: usize,
    snapshot: &ProjectSemanticSnapshot,
) -> Option<ScopeId> {
    let text_offset = TextSize(offset.min(u32::MAX as usize) as u32);

    if let Ok(view) = HirTreeView::try_new(&snapshot.hir) {
        if let Some(node) = find_node_at(doc.source_id, text_offset, snapshot) {
            for hir_node in snapshot.hir_nodes_for_syntax(node.id) {
                if !hir_node_can_own_completion_scope(*hir_node) {
                    continue;
                }
                if let Some(scope) = view.enclosing_scope(*hir_node) {
                    return Some(scope);
                }
            }
        }
    }

    fallback_scope_at_offset(doc.source_id, text_offset, snapshot)
}

fn hir_node_can_own_completion_scope(node: HirNodeRef) -> bool {
    matches!(
        node,
        HirNodeRef::Item(_)
            | HirNodeRef::Expr(_)
            | HirNodeRef::HandlerArm(_)
            | HirNodeRef::Stmt(_)
            | HirNodeRef::Pat(_)
            | HirNodeRef::Block(_)
    )
}

fn fallback_scope_at_offset(
    source_id: SourceId,
    offset: TextSize,
    snapshot: &ProjectSemanticSnapshot,
) -> Option<ScopeId> {
    let mut best = None;
    let mut best_len = u32::MAX;

    for scope in snapshot.scopes.iter() {
        if scope.span.source != source_id {
            continue;
        }
        let start = scope.span.range.start.0;
        let end = scope.span.range.end.0;
        if offset.0 >= start && offset.0 <= end {
            let len = end - start;
            if len < best_len {
                best_len = len;
                best = Some(scope.id);
            }
        }
    }

    best
}

fn collect_visible_scope_candidates(
    candidates: &mut Vec<Candidate>,
    snapshot: &ProjectSemanticSnapshot,
    start_scope: ScopeId,
    source_id: SourceId,
    offset: usize,
    context: CompletionContext,
) {
    let current_module = enclosing_hir_module(snapshot, start_scope);
    let current_part = current_module_part(snapshot, source_id);
    let mut current = Some(start_scope);

    while let Some(scope_id) = current {
        let Some(scope) = snapshot.scopes.get(scope_id) else {
            break;
        };

        for symbol_id in &scope.symbols {
            let Some(symbol) = snapshot.symbols.get(*symbol_id) else {
                continue;
            };
            if !scope_symbol_is_visible_at_cursor(snapshot, symbol, source_id, offset) {
                continue;
            }
            if !symbol_visible_from_current_module(symbol, current_module) {
                continue;
            }
            if !symbol_resolves_from_scope(snapshot, start_scope, symbol) {
                continue;
            }
            if let Some(candidate) =
                candidate_for_scope_symbol(snapshot, symbol, current_part, context)
            {
                candidates.push(candidate);
            }
        }

        current = scope.parent;
    }
}

fn enclosing_hir_module(
    snapshot: &ProjectSemanticSnapshot,
    start_scope: ScopeId,
) -> Option<HirModuleId> {
    let mut current = Some(start_scope);
    while let Some(scope_id) = current {
        let scope = snapshot.scopes.get(scope_id)?;
        if let ScopeOwner::Module(module) = scope.owner {
            return Some(module);
        }
        current = scope.parent;
    }
    None
}

fn current_module_part(
    snapshot: &ProjectSemanticSnapshot,
    source_id: SourceId,
) -> Option<ModulePartId> {
    snapshot
        .modules
        .parts
        .iter()
        .find_map(|(_, part)| (part.source == source_id).then_some(part.id))
}

fn symbol_visible_from_current_module(
    symbol: &Symbol,
    current_module: Option<HirModuleId>,
) -> bool {
    current_module.is_some_and(|module| symbol.defining_module == module)
        || symbol.visibility == Visibility::Public
}

fn candidate_for_scope_symbol(
    snapshot: &ProjectSemanticSnapshot,
    symbol: &Symbol,
    current_part: Option<ModulePartId>,
    context: CompletionContext,
) -> Option<Candidate> {
    if symbol.kind == SymbolKind::Import {
        return completion_for_import_symbol(snapshot, symbol, current_part, context);
    }

    let kind = completion_kind_for_scope_symbol(symbol.kind)?;
    if !completion_kind_allowed_in_context(context, kind) {
        return None;
    }
    Some(Candidate {
        item: CompletionItem {
            label: symbol.name.clone(),
            kind,
            detail: Some(completion_detail_for_symbol(snapshot, symbol)),
            insert_text: None,
        },
        priority: candidate_priority_for_symbol(symbol.kind),
    })
}

fn candidate_priority_for_symbol(kind: SymbolKind) -> CandidatePriority {
    match kind {
        SymbolKind::Local => CandidatePriority::Local,
        SymbolKind::Param => CandidatePriority::Param,
        SymbolKind::TypeParam => CandidatePriority::TypeParam,
        SymbolKind::Import => CandidatePriority::Import,
        _ => CandidatePriority::CurrentModule,
    }
}

fn completion_for_import_symbol(
    snapshot: &ProjectSemanticSnapshot,
    symbol: &Symbol,
    current_part: Option<ModulePartId>,
    context: CompletionContext,
) -> Option<Candidate> {
    let target_symbol = explicit_import_target_symbol(snapshot, symbol, current_part)
        .or_else(|| import_alias_target_symbol(snapshot, symbol));
    if let Some(target) = target_symbol {
        let kind = completion_kind_for_symbol(target.kind)?;
        if !completion_kind_allowed_in_context(context, kind) {
            return None;
        }
        return Some(Candidate {
            item: CompletionItem {
                label: symbol.name.clone(),
                kind,
                detail: Some(completion_detail_for_symbol(snapshot, target)),
                insert_text: None,
            },
            priority: CandidatePriority::Import,
        });
    }

    if explicit_import_is_module(snapshot, symbol, current_part)
        || import_alias_is_source_module(snapshot, symbol)
    {
        let kind = CompletionKind::Module;
        if !completion_kind_allowed_in_context(context, kind) {
            return None;
        }
        return Some(Candidate {
            item: CompletionItem {
                label: symbol.name.clone(),
                kind,
                detail: Some("imported module".to_string()),
                insert_text: None,
            },
            priority: CandidatePriority::Import,
        });
    }

    None
}

fn explicit_import_target_symbol<'a>(
    snapshot: &'a ProjectSemanticSnapshot,
    symbol: &Symbol,
    current_part: Option<ModulePartId>,
) -> Option<&'a Symbol> {
    let import = snapshot.resolved_imports.imports.iter().find(|import| {
        import.local_name == symbol.name && current_part.is_none_or(|part| import.from_part == part)
    })?;
    import_target_symbol(snapshot, &import.target)
}

fn explicit_import_is_module(
    snapshot: &ProjectSemanticSnapshot,
    symbol: &Symbol,
    current_part: Option<ModulePartId>,
) -> bool {
    snapshot.resolved_imports.imports.iter().any(|import| {
        import.local_name == symbol.name
            && current_part.is_none_or(|part| import.from_part == part)
            && matches!(import.target, ImportTarget::Module(_))
    })
}

fn import_target_symbol<'a>(
    snapshot: &'a ProjectSemanticSnapshot,
    target: &ImportTarget,
) -> Option<&'a Symbol> {
    match target {
        ImportTarget::SourceItem { item, name, .. } => {
            symbol_for_export_span(snapshot, item.span, name)
        }
        ImportTarget::Module(_)
        | ImportTarget::StdItem { .. }
        | ImportTarget::ExternalItem { .. } => None,
    }
}

fn import_alias_target_symbol<'a>(
    snapshot: &'a ProjectSemanticSnapshot,
    symbol: &Symbol,
) -> Option<&'a Symbol> {
    let SymbolDef::ImportAlias { path, .. } = &symbol.def else {
        return None;
    };
    let (name, module_path) = path.split_last()?;
    let module_id = source_module_for_path(snapshot, module_path)?;
    let module = snapshot.modules.modules.get(module_id)?;
    let export = module.visibility_exports.items.get(name)?;
    if export.visibility != Visibility::Public {
        return None;
    }
    symbol_for_export_span(snapshot, export.item.span, name)
}

fn import_alias_is_source_module(snapshot: &ProjectSemanticSnapshot, symbol: &Symbol) -> bool {
    let SymbolDef::ImportAlias { path, .. } = &symbol.def else {
        return false;
    };
    source_module_for_path(snapshot, path).is_some()
}

fn source_module_for_path(snapshot: &ProjectSemanticSnapshot, path: &[String]) -> Option<ModuleId> {
    snapshot
        .modules
        .by_path
        .get(&ModulePath {
            segments: path.to_vec(),
        })
        .copied()
}

fn symbol_for_export_span<'a>(
    snapshot: &'a ProjectSemanticSnapshot,
    span: etas_core::Span,
    name: &str,
) -> Option<&'a Symbol> {
    snapshot.symbols.iter().find(|symbol| {
        symbol.name == name
            && symbol.definition_span.source == span.source
            && symbol.definition_span.range.start.0 >= span.range.start.0
            && symbol.definition_span.range.end.0 <= span.range.end.0
    })
}

fn symbol_resolves_from_scope(
    snapshot: &ProjectSemanticSnapshot,
    scope: ScopeId,
    symbol: &Symbol,
) -> bool {
    let result = match symbol.kind {
        SymbolKind::Module => snapshot.scopes.lookup_module(scope, &symbol.name),
        _ => snapshot.scopes.lookup(scope, &symbol.name),
    };
    matches!(result, ResolveResult::Resolved(resolved) if resolved == symbol.id)
}

fn scope_symbol_is_visible_at_cursor(
    snapshot: &ProjectSemanticSnapshot,
    symbol: &Symbol,
    source_id: SourceId,
    offset: usize,
) -> bool {
    if symbol.definition_span.source == source_id
        && symbol.definition_span.range.end.0 > offset.min(u32::MAX as usize) as u32
    {
        return false;
    }

    match &symbol.def {
        SymbolDef::Local {
            initializer: Some(expr),
            ..
        }
        | SymbolDef::PatternBinding {
            initializer: Some(expr),
            ..
        } => snapshot
            .source_map
            .expr_sources
            .get(expr)
            .map(|origin| !span_contains_offset(origin.span(), source_id, offset))
            .unwrap_or(true),
        _ => true,
    }
}

fn span_contains_offset(span: etas_core::Span, source_id: SourceId, offset: usize) -> bool {
    span.source == source_id
        && offset.min(u32::MAX as usize) as u32 >= span.range.start.0
        && offset.min(u32::MAX as usize) as u32 <= span.range.end.0
}

fn completion_context(
    doc: &DocumentState,
    position: crate::service::LspPosition,
    snapshot: Option<&ProjectSemanticSnapshot>,
) -> CompletionContext {
    let offset = offset_of_position(&doc.text, position);

    if looks_like_type_context(&doc.text, offset) {
        return CompletionContext::Type;
    }

    if let Some(snapshot) = snapshot {
        let offset = TextSize(offset.min(u32::MAX as usize) as u32);
        if let Some(node) = find_node_at(doc.source_id, offset, snapshot) {
            let mut saw_expression = false;
            for hir_node in snapshot.hir_nodes_for_syntax(node.id) {
                match hir_node {
                    HirNodeRef::Type(_) => return CompletionContext::Type,
                    HirNodeRef::Expr(_) | HirNodeRef::Stmt(_) | HirNodeRef::Block(_) => {
                        saw_expression = true;
                    }
                    _ => {}
                }
            }
            if saw_expression {
                return CompletionContext::Expression;
            }
            match node.kind {
                SyntaxNodeKind::Type => return CompletionContext::Type,
                SyntaxNodeKind::Expr | SyntaxNodeKind::Stmt | SyntaxNodeKind::Block => {
                    return CompletionContext::Expression;
                }
                _ => {}
            }
        }
    }

    if is_top_level_position(&doc.text, offset) {
        CompletionContext::TopLevel
    } else if is_inside_braces(&doc.text, offset) {
        CompletionContext::Expression
    } else {
        CompletionContext::Unknown
    }
}

fn find_node_at(
    source_id: etas_core::SourceId,
    offset: TextSize,
    snapshot: &ProjectSemanticSnapshot,
) -> Option<SyntaxNodeRef> {
    let mut best: Option<SyntaxNodeRef> = None;
    let mut best_len: u32 = u32::MAX;

    for node in snapshot.source_map.syntax_nodes.values() {
        if node.span.source != source_id {
            continue;
        }
        let start = node.span.range.start.0;
        let end = node.span.range.end.0;
        if offset.0 >= start && offset.0 <= end {
            let len = end - start;
            if len < best_len {
                best_len = len;
                best = Some(*node);
            }
        }
    }

    best
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionContext {
    TopLevel,
    Expression,
    Type,
    Unknown,
}

const TOP_LEVEL_COMPLETION_KEYWORDS: &[&str] = &[
    "agent", "alias", "effect", "enum", "flow", "impl", "import", "let", "module", "policy",
    "protocol", "spec", "tool", "type",
];

const EXPRESSION_COMPLETION_KEYWORDS: &[&str] = &[
    "else", "for", "handler", "if", "in", "let", "match", "return", "where", "while",
];

const PRIMITIVE_TYPE_COMPLETIONS: &[&str] = &["i32", "string", "bool"];

const COMPLETION_KEYWORDS: &[&str] = &[
    "agent", "alias", "allow", "deny", "effect", "else", "enum", "flow", "for", "handler", "if",
    "impl", "import", "in", "let", "limit", "match", "module", "policy", "protocol", "require",
    "return", "spec", "tool", "type", "where", "while",
];

fn completion_keywords(context: CompletionContext) -> &'static [&'static str] {
    match context {
        CompletionContext::TopLevel => TOP_LEVEL_COMPLETION_KEYWORDS,
        CompletionContext::Expression => EXPRESSION_COMPLETION_KEYWORDS,
        CompletionContext::Type => &[],
        CompletionContext::Unknown => COMPLETION_KEYWORDS,
    }
}

fn completion_kind_allowed_in_context(context: CompletionContext, kind: CompletionKind) -> bool {
    match context {
        CompletionContext::TopLevel => false,
        CompletionContext::Expression => matches!(
            kind,
            CompletionKind::Flow
                | CompletionKind::Agent
                | CompletionKind::Tool
                | CompletionKind::Variable
                | CompletionKind::Module
        ),
        CompletionContext::Type => matches!(
            kind,
            CompletionKind::Type
                | CompletionKind::Enum
                | CompletionKind::Effect
                | CompletionKind::Protocol
                | CompletionKind::Spec
        ),
        CompletionContext::Unknown => true,
    }
}

fn looks_like_type_context(text: &str, offset: usize) -> bool {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_before_cursor = &text[line_start..offset];
    let trimmed = line_before_cursor.trim_end();
    if trimmed.ends_with(':') || trimmed.ends_with("->") {
        return true;
    }

    let last_colon = line_before_cursor.rfind(':');
    let last_arrow = line_before_cursor.rfind("->");
    let last_equals = line_before_cursor.rfind('=');
    let last_open_brace = line_before_cursor.rfind('{');
    let last_close_paren = line_before_cursor.rfind(')');

    if let Some(colon) = last_colon {
        let after_equals = last_equals.map(|i| colon > i).unwrap_or(true);
        let after_brace = last_open_brace.map(|i| colon > i).unwrap_or(true);
        if after_equals && after_brace {
            return true;
        }
    }

    if let Some(arrow) = last_arrow {
        let after_equals = last_equals.map(|i| arrow > i).unwrap_or(true);
        let after_close_paren = last_close_paren.map(|i| arrow > i).unwrap_or(false);
        if after_equals && after_close_paren {
            return true;
        }
    }

    false
}

fn is_top_level_position(text: &str, offset: usize) -> bool {
    !is_inside_braces(text, offset)
}

fn is_inside_braces(text: &str, offset: usize) -> bool {
    // Phase 2 intentionally uses a narrow textual heuristic only for incomplete code fallback.
    let offset = offset.min(text.len());
    let mut depth = 0usize;
    for ch in text[..offset].chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth > 0
}

fn completion_kind_for_symbol(kind: SymbolKind) -> Option<CompletionKind> {
    match kind {
        SymbolKind::Module => Some(CompletionKind::Module),
        SymbolKind::TypeAlias | SymbolKind::Type => Some(CompletionKind::Type),
        SymbolKind::Enum => Some(CompletionKind::Enum),
        SymbolKind::Spec => Some(CompletionKind::Spec),
        SymbolKind::Flow => Some(CompletionKind::Flow),
        SymbolKind::Agent => Some(CompletionKind::Agent),
        SymbolKind::Tool => Some(CompletionKind::Tool),
        SymbolKind::TopLevelLet => Some(CompletionKind::Variable),
        SymbolKind::Protocol => Some(CompletionKind::Protocol),
        SymbolKind::Effect => Some(CompletionKind::Effect),
        SymbolKind::EnumVariant
        | SymbolKind::EffectAction
        | SymbolKind::Param
        | SymbolKind::Local
        | SymbolKind::Field
        | SymbolKind::TypeParam
        | SymbolKind::EffectParam
        | SymbolKind::Import
        | SymbolKind::StdPreludeAlias => None,
    }
}

fn completion_kind_for_scope_symbol(kind: SymbolKind) -> Option<CompletionKind> {
    match kind {
        SymbolKind::Param | SymbolKind::Local => Some(CompletionKind::Variable),
        SymbolKind::TypeParam => Some(CompletionKind::Type),
        _ => completion_kind_for_symbol(kind),
    }
}

fn completion_detail_for_symbol(snapshot: &ProjectSemanticSnapshot, symbol: &Symbol) -> String {
    if let Some(detail) = completion_signature_detail(snapshot, symbol) {
        return detail;
    }

    match symbol.kind {
        SymbolKind::Param => "parameter".to_string(),
        SymbolKind::Local => "local".to_string(),
        SymbolKind::TypeParam => "type parameter".to_string(),
        _ => format!("{:?}", symbol.kind),
    }
}

fn completion_signature_detail(
    snapshot: &ProjectSemanticSnapshot,
    symbol: &Symbol,
) -> Option<String> {
    if !matches!(
        symbol.kind,
        SymbolKind::Flow | SymbolKind::Agent | SymbolKind::Tool | SymbolKind::TopLevelLet
    ) {
        return None;
    }

    let item = match &symbol.def {
        SymbolDef::Item { item } | SymbolDef::TopLevelLet { item, .. } => *item,
        _ => return None,
    };
    let signature = snapshot.types.item_signatures.get(&item)?;
    match signature {
        ItemSignature::Flow(signature) => Some(callable_signature_detail(
            &symbol.name,
            &signature.params,
            signature.output,
            &snapshot.type_store,
        )),
        ItemSignature::Agent(signature) => Some(callable_signature_detail(
            &symbol.name,
            &signature.params,
            signature.output,
            &snapshot.type_store,
        )),
        ItemSignature::Tool(signature) => Some(callable_signature_detail(
            &symbol.name,
            &signature.params,
            signature.output,
            &snapshot.type_store,
        )),
        ItemSignature::TopLevelLet(signature) => Some(format!(
            "{}: {}",
            symbol.name,
            type_name(signature.ty, &snapshot.type_store)
        )),
    }
}

fn callable_signature_detail(
    name: &str,
    params: &[TypeId],
    output: TypeId,
    store: &TypeStore,
) -> String {
    let params = params
        .iter()
        .map(|param| type_name(*param, store))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{name}({params}) -> {}", type_name(output, store))
}

fn type_name(ty: TypeId, store: &TypeStore) -> String {
    match store.get(ty).cloned() {
        Some(Type::Primitive(primitive)) => primitive.source_name().to_owned(),
        Some(Type::IntegerLiteral { .. }) => "i32".to_owned(),
        Some(Type::Var(var)) => format!("T{}", var.0),
        Some(Type::Array(inner)) => format!("Array[{}]", type_name(inner, store)),
        Some(Type::List(inner)) => format!("List[{}]", type_name(inner, store)),
        Some(Type::Map { key, value }) => {
            format!(
                "Map[{}, {}]",
                type_name(key, store),
                type_name(value, store)
            )
        }
        Some(Type::Set(inner)) => format!("Set[{}]", type_name(inner, store)),
        Some(Type::Range { index }) => format!("Range[{}]", type_name(index, store)),
        Some(Type::Slice(inner)) => format!("Slice[{}]", type_name(inner, store)),
        Some(Type::Option(inner)) => format!("Option[{}]", type_name(inner, store)),
        Some(Type::Result { ok, err }) => {
            format!(
                "Result[{}, {}]",
                type_name(ok, store),
                type_name(err, store)
            )
        }
        Some(Type::Record(record)) => {
            let fields = record
                .fields
                .iter()
                .map(|field| format!("{}: {}", field.name, type_name(field.ty, store)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{fields}}}")
        }
        Some(Type::Tuple(elems)) => {
            let elems = elems
                .iter()
                .map(|elem| type_name(*elem, store))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({elems})")
        }
        Some(Type::Enum(enum_ref)) => enum_ref.name,
        Some(Type::Function(flow)) => {
            let input = flow
                .input
                .iter()
                .map(|input| type_name(*input, store))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({input}) -> {}", type_name(flow.output, store))
        }
        Some(Type::Handler(_)) => "handler".to_owned(),
        Some(Type::Named(named)) => named.name,
        Some(Type::Nominal(nominal)) => nominal.name,
        Some(Type::Applied { constructor, args }) => {
            let args = args
                .iter()
                .map(|arg| type_name(*arg, store))
                .collect::<Vec<_>>()
                .join(", ");
            format!("type{}[{args}]", constructor.0)
        }
        Some(Type::Refined { base, .. }) => type_name(base, store),
        Some(Type::Trust { wrapper, inner }) => format!("{wrapper}[{}]", type_name(inner, store)),
        Some(Type::Schema(inner)) => format!("Schema[{}]", type_name(inner, store)),
        Some(Type::Prompt) => "Prompt".to_owned(),
        Some(Type::PromptPart) => "PromptPart".to_owned(),
        Some(Type::Message(inner)) => format!("Message[{}]", type_name(inner, store)),
        Some(Type::MemorySelection(inner)) => {
            format!("MemorySelection[{}]", type_name(inner, store))
        }
        Some(Type::Store { key, value }) => {
            format!(
                "Store[{}, {}]",
                type_name(key, store),
                type_name(value, store)
            )
        }
        Some(Type::MemoryPlace(place)) => place.segments.join("."),
        Some(Type::MemoryRegion(inner)) => format!("MemoryRegion[{}]", type_name(inner, store)),
        Some(Type::ResourceHandle(handle)) => match handle {
            etas_types::ResourceHandleType::MemoryRegion { schema } => {
                format!("ResourceHandleMemoryRegion[{}]", type_name(schema, store))
            }
            etas_types::ResourceHandleType::ExternalTool { signature } => {
                format!("ExternalTool[{}]", type_name(signature, store))
            }
            etas_types::ResourceHandleType::Other { name, args } => {
                let args = args
                    .iter()
                    .map(|arg| type_name(*arg, store))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}[{args}]")
            }
        },
        None => "<unknown>".to_owned(),
    }
}
