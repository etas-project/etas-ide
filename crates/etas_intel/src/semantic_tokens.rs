use std::collections::HashMap;

use etas_core::{Severity, SourceId, Span, TextRange};
use etas_frontend::{
    AstItemKind, HirExprPathResolution, HirPathResolution, ImportTarget, ParseOutput,
    ProjectSemanticSnapshot, ResolvedModuleTarget,
};
use etas_hir::{
    HirExpr, HirFieldInit, HirItem, HirType, ResolveResult, Symbol, SymbolDef, SymbolKind,
};
use etas_std::{StdDecl, StdSymbol, StdSymbolKind, TypeDeclKind};
use etas_syntax::ast::ImportTree;
use etas_types::{SymbolTypeFact, Type, TypeId};

use crate::IntelSemanticToken;

pub(crate) fn collect(
    snapshot: &ProjectSemanticSnapshot,
    source: SourceId,
    parsed: &ParseOutput,
) -> Vec<IntelSemanticToken> {
    let classifier = Classifier::new(snapshot, source);
    let mut tokens = Vec::new();
    for expr in snapshot.types.agent_inferences.keys() {
        let Some(HirExpr::Infer { span, .. }) = snapshot.hir.exprs.get(*expr) else {
            continue;
        };
        if span.source == source {
            if let Some(range) = crate::syntax::identifier_range(parsed, *span, "infer") {
                tokens.push(token(Span::new(source, range), 9));
            }
        }
    }
    for (id, origin) in &snapshot.source_map.symbol_sources {
        let span = origin.span();
        if span.source != source {
            continue;
        }
        let Some(symbol) = snapshot.symbols.get(*id) else {
            continue;
        };
        if matches!(
            symbol.kind,
            SymbolKind::Import | SymbolKind::StdPreludeAlias
        ) {
            continue;
        }
        let Some(kind) = classifier.symbol(symbol) else {
            continue;
        };
        let name = if symbol.kind == SymbolKind::EffectAction {
            symbol.name.rsplit('.').next().unwrap_or(&symbol.name)
        } else {
            &symbol.name
        };
        if let Some(range) = crate::syntax::identifier_range(parsed, symbol.definition_span, name) {
            let mut token = token(Span::new(source, range), kind);
            token.modifier_bits = u32::from(is_declaration(symbol.kind));
            tokens.push(token);
        }
    }

    // Import syntax supplies exact name/alias spans; checked targets supply kinds.
    for import in &parsed.parsed.value.imports {
        let resolved_kind = |local: &str| {
            snapshot
                .resolved_imports
                .imports
                .iter()
                .find(|resolved| {
                    resolved.import.source == source
                        && resolved.import.span.range == import.span.range
                        && resolved.local_name == local
                })
                .and_then(|resolved| classifier.target(&resolved.target))
        };
        match &import.tree {
            ImportTree::Single { path, alias, .. } => {
                let Some(last) = path.segments.last() else {
                    continue;
                };
                let kind = resolved_kind(
                    alias
                        .as_ref()
                        .map(|name| name.text.as_str())
                        .unwrap_or(&last.text),
                );
                for segment in &path.segments[..path.segments.len() - 1] {
                    tokens.push(token(segment.span, 5));
                }
                if let Some(kind) = kind {
                    tokens.push(token(last.span, kind));
                    if let Some(alias) = alias {
                        tokens.push(token(alias.span, kind));
                    }
                }
            }
            ImportTree::Group { prefix, items, .. } => {
                for segment in &prefix.segments {
                    tokens.push(token(segment.span, 5));
                }
                for item in items {
                    let name = item.alias.as_ref().unwrap_or(&item.name);
                    if let Some(kind) = resolved_kind(&name.text) {
                        tokens.push(token(item.name.span, kind));
                        if let Some(alias) = &item.alias {
                            tokens.push(token(alias.span, kind));
                        }
                    }
                }
            }
            ImportTree::Wildcard { prefix, .. } => {
                for segment in &prefix.segments {
                    tokens.push(token(segment.span, 5));
                }
            }
            ImportTree::Error { .. } => {}
        }
    }

    for resolved in &snapshot.resolved_paths.expr_paths {
        if resolved.path.span.source != source {
            continue;
        }
        if let Some(kind) = classifier.expression(resolved) {
            if let Some(segment) = resolved.path.segments.last() {
                tokens.push(token(segment.span, kind));
            }
        }
    }

    // Project actual type paths, not all identifiers in a broad type origin.
    // Record field labels and refinement expressions have different semantics.
    for (_, ty) in snapshot.hir.types.iter() {
        if ty.span().source != source {
            continue;
        }
        match ty {
            HirType::Path { path, .. } => {
                let kind = match path.resolution {
                    ResolveResult::Resolved(id) => snapshot
                        .symbols
                        .get(id)
                        .and_then(|symbol| classifier.symbol(symbol))
                        .unwrap_or(1),
                    _ => 1,
                };
                for (index, segment) in path.segments.iter().enumerate() {
                    tokens.push(token(
                        segment.span,
                        if index + 1 == path.segments.len() {
                            kind
                        } else {
                            5
                        },
                    ));
                }
            }
            HirType::Record { fields, .. } => {
                for field in fields {
                    let Some(symbol) = snapshot.symbols.get(field.symbol) else {
                        continue;
                    };
                    if let Some(range) =
                        crate::syntax::identifier_range(parsed, field.span, &symbol.name)
                    {
                        tokens.push(token(Span::new(source, range), 6));
                    }
                }
            }
            _ => {}
        }
    }
    for (id, expr) in snapshot.hir.exprs.iter() {
        if expr.span(&snapshot.hir.blocks).source != source {
            continue;
        }
        if matches!(expr, HirExpr::Call { .. } | HirExpr::MethodCall { .. })
            && snapshot.types.expr_types.contains_key(&id)
        {
            let span = expr.span(&snapshot.hir.blocks);
            let invalid = snapshot.diagnostics.diagnostics.iter().any(|diagnostic| {
                let error = diagnostic.primary.span;
                diagnostic.severity == Severity::Error
                    && error.source == source
                    && error.range.start < span.range.end
                    && error.range.end > span.range.start
            });
            if !invalid {
                match expr {
                    HirExpr::Call { callee, .. } => {
                        if let HirExpr::Path(path) = &snapshot.hir.exprs[*callee] {
                            if let ResolveResult::PartiallyResolved(partial) = &path.resolution {
                                let receiver = partial
                                    .resolved_prefix
                                    .and_then(|symbol| snapshot.symbols.get(symbol))
                                    .and_then(|symbol| classifier.symbol(symbol));
                                if let Some(kind @ (3 | 4 | 6)) = receiver {
                                    if let Some(segment) = path
                                        .segments
                                        .get(partial.resolved_segments.saturating_sub(1) as usize)
                                    {
                                        tokens.push(token(segment.span, kind));
                                    }
                                    if let Some(segment) = path.segments.last() {
                                        tokens.push(token(segment.span, 0));
                                    }
                                }
                            }
                        }
                    }
                    HirExpr::MethodCall {
                        receiver, method, ..
                    } => {
                        // Exclude the receiver: it may have the same name, or
                        // contain another call to this method in a chain.
                        let receiver_end = snapshot.hir.exprs[*receiver]
                            .span(&snapshot.hir.blocks)
                            .range
                            .end;
                        let name_span =
                            Span::new(source, TextRange::new(receiver_end, span.range.end));
                        if let Some(range) =
                            crate::syntax::identifier_range(parsed, name_span, method)
                        {
                            tokens.push(token(Span::new(source, range), 0));
                        }
                    }
                    _ => {}
                }
            }
        }
        if let HirExpr::Record(record) = expr {
            let Some(ty) = snapshot.types.expr_types.get(&id) else {
                continue;
            };
            let is_enum = match snapshot.type_store.get(*ty) {
                Some(Type::Enum(_)) => true,
                Some(Type::Applied { constructor, .. }) => matches!(
                    snapshot.type_store.get(TypeId(constructor.0)),
                    Some(Type::Enum(_))
                ),
                _ => false,
            };
            // Record constructor paths are not ordinary path expressions.
            // The checked HIR identifies the name and each field's exact role.
            if let Some(path) = &record.path {
                for (index, segment) in path.segments.iter().enumerate() {
                    tokens.push(token(
                        segment.span,
                        if index + 1 == path.segments.len() {
                            if is_enum { 2 } else { 1 }
                        } else if is_enum && index + 2 == path.segments.len() {
                            1
                        } else {
                            5
                        },
                    ));
                }
            }
            for field in &record.fields {
                let (name, span, kind) = match field {
                    HirFieldInit::Named { name, span, .. } => (name, *span, Some(6)),
                    HirFieldInit::Shorthand {
                        name,
                        resolution,
                        span,
                    } => {
                        let kind = match resolution {
                            ResolveResult::Resolved(symbol) => snapshot
                                .symbols
                                .get(*symbol)
                                .and_then(|symbol| classifier.symbol(symbol)),
                            _ => None,
                        };
                        (name, *span, kind)
                    }
                };
                if let Some(kind) = kind {
                    if let Some(range) = crate::syntax::identifier_range(parsed, span, name) {
                        tokens.push(token(Span::new(source, range), kind));
                    }
                }
            }
        }
        let HirExpr::Perform { action, .. } = expr else {
            continue;
        };
        if action.span.source != source {
            continue;
        }
        for segment in &action.effect.path.segments {
            tokens.push(token(segment.span, 8));
        }
        if let Some(range) = crate::syntax::identifier_range(parsed, action.span, &action.action) {
            tokens.push(token(Span::new(source, range), 0));
        }
    }
    tokens
}

fn token(span: Span, kind: u32) -> IntelSemanticToken {
    IntelSemanticToken {
        byte_start: span.range.start.0,
        byte_end: span.range.end.0,
        token_type: kind,
        modifier_bits: 0,
    }
}

pub(crate) fn member_name_span(
    snapshot: &ProjectSemanticSnapshot,
    expr: etas_hir::HirExprId,
    parsed: &ParseOutput,
) -> Option<Span> {
    match snapshot.hir.exprs.get(expr)? {
        HirExpr::Call { callee, .. } => {
            let HirExpr::Path(path) = snapshot.hir.exprs.get(*callee)? else {
                return None;
            };
            (path.segments.len() > 1).then(|| path.segments.last().unwrap().span)
        }
        HirExpr::MethodCall {
            receiver,
            method,
            span,
            ..
        } => {
            let receiver_end = snapshot.hir.exprs[*receiver]
                .span(&snapshot.hir.blocks)
                .range
                .end;
            crate::syntax::identifier_range(
                parsed,
                Span::new(span.source, TextRange::new(receiver_end, span.range.end)),
                method,
            )
            .map(|range| Span::new(span.source, range))
        }
        _ => None,
    }
}

struct Classifier<'a> {
    snapshot: &'a ProjectSemanticSnapshot,
    source: SourceId,
    external_bindings: HashMap<Vec<String>, u32>,
}

impl<'a> Classifier<'a> {
    fn new(snapshot: &'a ProjectSemanticSnapshot, source: SourceId) -> Self {
        let external_bindings = snapshot
            .symbols
            .iter()
            .filter_map(|symbol| {
                if symbol.definition_span.source != source {
                    return None;
                }
                let SymbolDef::ImportAlias { path, .. } = &symbol.def else {
                    return None;
                };
                let kind = fact_kind(snapshot.types.symbol_types.get(&symbol.id)?)?;
                Some((path.clone(), kind))
            })
            .collect();
        Self {
            snapshot,
            source,
            external_bindings,
        }
    }

    fn symbol(&self, symbol: &Symbol) -> Option<u32> {
        if !matches!(
            symbol.kind,
            SymbolKind::Import | SymbolKind::StdPreludeAlias
        ) {
            return symbol_kind(symbol.kind);
        }
        if let SymbolDef::ImportAlias { path, .. } = &symbol.def {
            if let Some(symbol) = self.snapshot.std_registry.lookup_qualified(path) {
                return Some(std_kind(symbol));
            }
        }
        self.snapshot
            .resolved_imports
            .imports
            .iter()
            .find(|import| {
                import.import.source == symbol.definition_span.source
                    && import.local_name == symbol.name
                    && import.import.span.range.start <= symbol.definition_span.range.start
                    && import.import.span.range.end >= symbol.definition_span.range.end
            })
            .and_then(|import| self.target(&import.target))
            .or_else(|| {
                self.snapshot
                    .types
                    .symbol_types
                    .get(&symbol.id)
                    .and_then(fact_kind)
            })
    }

    fn target(&self, target: &ImportTarget) -> Option<u32> {
        match target {
            ImportTarget::Module(_) => Some(5),
            ImportTarget::SourceItem { item, .. } => item_kind(item.kind),
            ImportTarget::StdItem { symbol, .. } => {
                self.snapshot.std_registry.symbol(*symbol).map(std_kind)
            }
            ImportTarget::ExternalItem {
                module_path, name, ..
            } => {
                let mut path = module_path.segments.clone();
                path.push(name.clone());
                self.external_bindings.get(&path).copied()
            }
        }
    }

    fn export(&self, module: &ResolvedModuleTarget, name: &str) -> Option<u32> {
        match module {
            ResolvedModuleTarget::Source { module, .. } => {
                let item = &self
                    .snapshot
                    .modules
                    .modules
                    .get(*module)?
                    .visibility_exports
                    .items
                    .get(name)?
                    .item;
                item_kind(item.kind)
            }
            ResolvedModuleTarget::Std { module, .. } => self
                .snapshot
                .std_registry
                .module_exports(*module)
                .find(|(export, _)| *export == name)
                .map(|(_, symbol)| std_kind(symbol)),
            ResolvedModuleTarget::External { path, .. } => {
                let mut path = path.segments.clone();
                path.push(name.to_owned());
                self.external_bindings.get(&path).copied()
            }
        }
    }

    fn expression(&self, resolved: &HirExprPathResolution) -> Option<u32> {
        if resolved.path.span.source != self.source {
            return None;
        }
        match &resolved.result {
            HirPathResolution::DirectSymbol(id) => self.symbol(self.snapshot.symbols.get(*id)?),
            HirPathResolution::ExplicitImport { target } => self.target(target),
            HirPathResolution::WildcardImport { target, .. } => {
                self.export(target, &resolved.path.segments.last()?.name)
            }
            HirPathResolution::PartialImport {
                target, remaining, ..
            } => match (target, remaining.as_slice()) {
                (ImportTarget::Module(module), [name]) => self.export(module, name),
                (_, []) => self.target(target),
                _ => None,
            },
            HirPathResolution::PartialSymbol {
                prefix, remaining, ..
            } if remaining.len() == 1 => {
                let symbol = self.snapshot.symbols.get(*prefix)?;
                let HirItem::Enum(declaration) =
                    self.snapshot.hir.items.get(symbol.defining_item?)?
                else {
                    return None;
                };
                declaration
                    .variants
                    .iter()
                    .filter_map(|variant| self.snapshot.symbols.get(variant.symbol))
                    .find(|symbol| symbol.name == remaining[0])
                    .and_then(|symbol| self.symbol(symbol))
            }
            _ => None,
        }
    }
}

fn fact_kind(fact: &SymbolTypeFact) -> Option<u32> {
    Some(match fact {
        SymbolTypeFact::Flow { .. }
        | SymbolTypeFact::Agent { .. }
        | SymbolTypeFact::Tool { .. }
        | SymbolTypeFact::EffectAction { .. } => 0,
        SymbolTypeFact::Type { .. }
        | SymbolTypeFact::TypeAlias { .. }
        | SymbolTypeFact::NominalType { .. } => 1,
        SymbolTypeFact::Param { .. } => 3,
        SymbolTypeFact::Local { .. }
        | SymbolTypeFact::Value { .. }
        | SymbolTypeFact::TopLevelLet { .. } => 4,
        SymbolTypeFact::Field { .. } => 6,
        SymbolTypeFact::Effect { .. } | SymbolTypeFact::Spec { .. } => 8,
        SymbolTypeFact::Error => return None,
    })
}

fn std_kind(symbol: &StdSymbol) -> u32 {
    match symbol.kind {
        StdSymbolKind::Flow | StdSymbolKind::Tool | StdSymbolKind::EffectAction => 0,
        StdSymbolKind::Type => match &symbol.decl {
            StdDecl::Type(decl) if decl.kind == TypeDeclKind::Spec => 8,
            _ => 1,
        },
        StdSymbolKind::Constructor => 2,
        StdSymbolKind::Effect | StdSymbolKind::Requirement => 8,
        StdSymbolKind::Value => 4,
        StdSymbolKind::Module => 5,
    }
}

fn item_kind(kind: AstItemKind) -> Option<u32> {
    Some(match kind {
        AstItemKind::Flow | AstItemKind::Tool | AstItemKind::Agent => 0,
        AstItemKind::TypeAlias | AstItemKind::Type | AstItemKind::Enum => 1,
        AstItemKind::TopLevelLet => 4,
        AstItemKind::Spec | AstItemKind::Protocol | AstItemKind::Effect => 8,
        AstItemKind::Impl | AstItemKind::Error => return None,
    })
}

fn symbol_kind(kind: SymbolKind) -> Option<u32> {
    Some(match kind {
        SymbolKind::Flow | SymbolKind::Tool | SymbolKind::Agent | SymbolKind::EffectAction => 0,
        SymbolKind::TypeAlias | SymbolKind::Type | SymbolKind::Enum => 1,
        SymbolKind::EnumVariant => 2,
        SymbolKind::Param => 3,
        SymbolKind::Local | SymbolKind::TopLevelLet => 4,
        SymbolKind::Module => 5,
        SymbolKind::Field => 6,
        SymbolKind::TypeParam | SymbolKind::EffectParam => 7,
        SymbolKind::Spec | SymbolKind::Protocol | SymbolKind::Effect => 8,
        SymbolKind::Import | SymbolKind::StdPreludeAlias => return None,
    })
}

fn is_declaration(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Flow
            | SymbolKind::Type
            | SymbolKind::TypeAlias
            | SymbolKind::Enum
            | SymbolKind::Spec
            | SymbolKind::Module
            | SymbolKind::Effect
            | SymbolKind::Tool
            | SymbolKind::Agent
            | SymbolKind::Protocol
    )
}
