use etas_core::{SourceId, Span, TextRange};
use etas_frontend::{
    DefinitionTarget, ImportTarget, ProjectSemanticSnapshot, ResolvedModuleTarget,
};
use etas_hir::{HirExpr, HirItem, HirType, ResolveResult, Symbol, SymbolKind};
use etas_syntax::TokenKind;
use etas_types::{
    CallableGenericParam, CallableGenericParamKind, CallableSignature, CheckedSpecRef,
    EffectArgRef, EffectRowRef, ItemSignature, SymbolTypeFact, Type, TypeId, TypeStore,
};

use crate::service::{DocumentState, HoverResult};

pub(crate) fn hover(
    snapshot: &ProjectSemanticSnapshot,
    doc: &DocumentState,
    offset: usize,
) -> Option<HoverResult> {
    let checked_source = snapshot
        .sources
        .files
        .iter()
        .find(|file| file.id == doc.source_id)?;
    // Last-good facts must not be attached to unrelated identifiers after an edit.
    if checked_source.text.as_ref() != doc.text {
        return None;
    }
    let parsed = crate::syntax::parse(&doc.text);
    let token = parsed.parsed.tokens.tokens.iter().find(|token| {
        (token.span.range.start.0 as usize..token.span.range.end.0 as usize).contains(&offset)
    })?;
    if !token.kind.is_ident_like()
        && !matches!(
            token.kind,
            TokenKind::IntLit | TokenKind::FloatLit | TokenKind::StringLit | TokenKind::CharLit
        )
    {
        return None;
    }
    let range = token.span.range;
    let name = &doc.text[range.start.0 as usize..range.end.0 as usize];
    let printer = Printer {
        snapshot,
        generics: &[],
    };

    let declaration = snapshot
        .source_map
        .symbol_sources
        .iter()
        .filter_map(|(id, origin)| {
            let span = origin.span();
            let symbol = snapshot.symbols.get(*id)?;
            (span.source == doc.source_id
                && crate::syntax::identifier_range(&parsed, symbol.definition_span, &symbol.name)
                    == Some(range))
            .then_some((span.range.end.0 - span.range.start.0, symbol))
        })
        .min_by_key(|(len, _)| *len);

    let text = declaration
        .and_then(|(_, symbol)| printer.symbol(symbol))
        .or_else(|| {
            snapshot.hir.exprs.iter().find_map(|(id, expr)| {
                let HirExpr::Record(record) = expr else {
                    return None;
                };
                let path = record.path.as_ref()?;
                let segment = path.segments.last()?;
                if segment.span.source != doc.source_id
                    || segment.span.range != range
                    || !snapshot.types.expr_types.contains_key(&id)
                {
                    return None;
                }
                let ResolveResult::Resolved(symbol) = path.resolution else {
                    return None;
                };
                printer.symbol(snapshot.symbols.get(symbol)?)
            })
        })
        .or_else(|| {
            let resolved = snapshot
                .resolved_paths
                .expr_paths
                .iter()
                .filter(|resolved| contains(resolved.path.span, doc.source_id, range))
                .min_by_key(|resolved| {
                    resolved.path.span.range.end.0 - resolved.path.span.range.start.0
                })?;
            // Module prefixes must not display the final item's signature.
            if resolved.path.segments.last()?.name != name {
                return None;
            }
            let target = snapshot.definition_target_for_expr(resolved.expr);
            let symbol = target.and_then(|target| match target {
                DefinitionTarget::SourceSymbol { symbol, .. } => snapshot.symbols.get(*symbol),
                DefinitionTarget::Import(target) => import_symbol(snapshot, target, name),
            });
            if let Some(text) = symbol.and_then(|symbol| printer.symbol(symbol)) {
                return Some(text);
            }
            let ty = *snapshot.types.expr_types.get(&resolved.expr)?;
            let mut text = if let Some(Type::Function(flow)) = snapshot.type_store.get(ty) {
                code(&printer.callable(
                    "flow",
                    name,
                    &CallableSignature {
                        generic_params: vec![],
                        params: flow.input.clone(),
                        output: flow.output,
                        effects: flow.effects.clone(),
                        requested_actions: None,
                    },
                    &[],
                ))
            } else {
                code(&format!("{name}: {}", printer.ty(ty)))
            };
            if let Some(DefinitionTarget::Import(ImportTarget::StdItem { symbol, .. })) = target {
                if let Some(item) = snapshot.std_registry.symbol(*symbol) {
                    if !item.docs.summary.is_empty() {
                        text.push_str(&format!("\n\n{}", item.docs.summary));
                    }
                }
            }
            Some(text)
        })
        .or_else(|| {
            let (id, _) = snapshot
                .source_map
                .type_sources
                .iter()
                .filter(|(_, origin)| contains(origin.span(), doc.source_id, range))
                .min_by_key(|(_, origin)| {
                    origin.span().range.end.0 - origin.span().range.start.0
                })?;
            if let Some(HirType::Path { path, .. }) = snapshot.hir.types.get(*id) {
                if let ResolveResult::Resolved(symbol) = path.resolution {
                    if let Some(text) = snapshot
                        .symbols
                        .get(symbol)
                        .and_then(|symbol| printer.symbol(symbol))
                    {
                        return Some(text);
                    }
                }
            }
            snapshot.types.type_refs.get(id).map(|ty| {
                if let Some(Type::Record(record)) = snapshot.type_store.get(*ty) {
                    if let Some(field) = record.fields.iter().find(|field| field.name == name) {
                        return code(&format!("{name}: {}", printer.ty(field.ty)));
                    }
                }
                code(&printer.ty(*ty))
            })
        })
        .or_else(|| {
            let (id, _) = snapshot
                .source_map
                .expr_sources
                .iter()
                .filter(|(id, origin)| {
                    contains(origin.span(), doc.source_id, range)
                        && snapshot.types.expr_types.contains_key(id)
                })
                .min_by_key(|(_, origin)| {
                    origin.span().range.end.0 - origin.span().range.start.0
                })?;
            let ty = printer.ty(*snapshot.types.expr_types.get(id)?);
            match snapshot.hir.exprs.get(*id) {
                Some(HirExpr::Field { field, .. }) if field == name => {
                    Some(code(&format!("{field}: {ty}")))
                }
                _ if !token.kind.is_ident_like() => Some(code(&ty)),
                _ => None,
            }
        })?;

    Some(HoverResult {
        text,
        byte_start: range.start.0 as usize,
        byte_end: range.end.0 as usize,
        source_text: doc.text.clone(),
    })
}

fn contains(span: Span, source: SourceId, range: TextRange) -> bool {
    span.source == source && span.range.start <= range.start && span.range.end >= range.end
}

fn import_symbol<'a>(
    snapshot: &'a ProjectSemanticSnapshot,
    target: &ImportTarget,
    name: &str,
) -> Option<&'a Symbol> {
    let (span, name) = match target {
        ImportTarget::SourceItem { item, name, .. } => (item.span, name.as_str()),
        ImportTarget::Module(ResolvedModuleTarget::Source { module, .. }) => {
            let export = snapshot
                .modules
                .modules
                .get(*module)?
                .visibility_exports
                .items
                .get(name)?;
            (export.item.span, name)
        }
        _ => return None,
    };
    snapshot.symbols.iter().find(|symbol| {
        symbol.name == name
            && contains(
                span,
                symbol.definition_span.source,
                symbol.definition_span.range,
            )
    })
}

fn code(text: &str) -> String {
    format!("```etas\n{text}\n```")
}

struct Printer<'a> {
    snapshot: &'a ProjectSemanticSnapshot,
    generics: &'a [CallableGenericParam],
}

impl Printer<'_> {
    fn symbol(&self, symbol: &Symbol) -> Option<String> {
        let snapshot = self.snapshot;
        if matches!(
            symbol.kind,
            SymbolKind::Import | SymbolKind::StdPreludeAlias
        ) {
            let import = snapshot.resolved_imports.imports.iter().find(|import| {
                import.local_name == symbol.name
                    && import.span.source == symbol.definition_span.source
            })?;
            return self.symbol(import_symbol(snapshot, &import.target, &symbol.name)?);
        }
        let signature = snapshot.types.symbol_types.get(&symbol.id);
        let item = symbol
            .defining_item
            .and_then(|id| snapshot.hir.items.get(id));
        let item_signature = symbol
            .defining_item
            .and_then(|id| snapshot.types.item_signatures.get(&id));
        let callable = match signature {
            Some(
                SymbolTypeFact::Flow { signature }
                | SymbolTypeFact::Agent { signature }
                | SymbolTypeFact::Tool { signature },
            ) => Some(signature),
            _ => match item_signature {
                Some(
                    ItemSignature::Flow(signature)
                    | ItemSignature::Agent(signature)
                    | ItemSignature::Tool(signature),
                ) if matches!(
                    symbol.kind,
                    SymbolKind::Flow | SymbolKind::Agent | SymbolKind::Tool
                ) =>
                {
                    Some(signature)
                }
                _ => None,
            },
        };
        if let Some(signature) = callable {
            let keyword = match symbol.kind {
                SymbolKind::Agent => "agent",
                SymbolKind::Tool => "tool",
                _ => "flow",
            };
            let params = match item {
                Some(HirItem::Flow(item)) => item.params.as_slice(),
                Some(HirItem::Agent(item)) => item.params.as_slice(),
                Some(HirItem::Tool(item)) => item.params.as_slice(),
                _ => &[],
            };
            let names = params
                .iter()
                .filter_map(|id| snapshot.symbols.get(*id).map(|symbol| symbol.name.clone()))
                .collect::<Vec<_>>();
            let printer = Printer {
                snapshot,
                generics: &signature.generic_params,
            };
            let mut declaration = printer.callable(keyword, &symbol.name, signature, &names);
            let summary = snapshot.effects.symbol_effects.get(&symbol.id);
            if signature.effects.is_none() {
                if let Some(row) =
                    summary.and_then(|summary| printer.solved_row(&summary.escaping_effects))
                {
                    declaration.push_str(&format!(" {row}"));
                }
            }
            let mut text = code(&declaration);
            if let Some(summary) = summary {
                if !summary.requested_actions.is_empty() {
                    if let Some(row) = printer.solved_row(&summary.requested_actions) {
                        text.push_str(&format!("\n\nRequested actions:\n\n{}", code(&row)));
                    } else {
                        text.push_str("\n\nRequested actions: open or unresolved row.");
                    }
                }
            }
            return Some(text);
        }
        let text = match signature {
            Some(
                SymbolTypeFact::Param { ty }
                | SymbolTypeFact::Field { ty }
                | SymbolTypeFact::Value { ty },
            ) => format!("{}: {}", symbol.name, self.ty(*ty)),
            Some(SymbolTypeFact::Local { ty, mutable }) => format!(
                "let {}{}: {}",
                if *mutable { "mut " } else { "" },
                symbol.name,
                self.ty(*ty)
            ),
            Some(SymbolTypeFact::TopLevelLet { ty, .. }) => {
                format!("let {}: {}", symbol.name, self.ty(*ty))
            }
            Some(SymbolTypeFact::TypeAlias { target, params }) => format!(
                "alias {}{} = {}",
                symbol.name,
                type_params(params),
                self.ty(*target)
            ),
            Some(SymbolTypeFact::NominalType {
                params,
                representation,
                ..
            }) => {
                let repr = representation
                    .map(|ty| format!(" = {}", self.ty(ty)))
                    .unwrap_or_default();
                format!("type {}{}{repr}", symbol.name, type_params(params))
            }
            Some(SymbolTypeFact::EffectAction { signature }) => {
                return Some(code(&self.callable(
                    "action",
                    &symbol.name,
                    &CallableSignature {
                        generic_params: signature.generic_params.clone(),
                        params: signature.params.clone(),
                        output: signature.output,
                        effects: None,
                        requested_actions: None,
                    },
                    &[],
                )));
            }
            _ => match symbol.kind {
                SymbolKind::Type | SymbolKind::Enum => format!(
                    "{} {}",
                    if symbol.kind == SymbolKind::Enum {
                        "enum"
                    } else {
                        "type"
                    },
                    symbol.name
                ),
                SymbolKind::Spec => format!("spec {}", symbol.name),
                SymbolKind::Effect => format!("effect {}", symbol.name),
                SymbolKind::Module => format!("module {}", symbol.name),
                SymbolKind::TypeParam => symbol.name.clone(),
                _ => return None,
            },
        };
        Some(code(&text))
    }

    fn callable(
        &self,
        keyword: &str,
        name: &str,
        signature: &CallableSignature,
        names: &[String],
    ) -> String {
        let printer = Printer {
            snapshot: self.snapshot,
            generics: &signature.generic_params,
        };
        let generics = signature
            .generic_params
            .iter()
            .map(|param| {
                let name = match param.kind {
                    CallableGenericParamKind::Effect => format!("effect {}", param.name),
                    CallableGenericParamKind::Type => param.name.clone(),
                };
                let bounds = param
                    .bounds
                    .iter()
                    .map(|bound| {
                        let name = match &bound.spec {
                            CheckedSpecRef::Source(id) => self
                                .snapshot
                                .symbols
                                .get(*id)
                                .map(|symbol| symbol.name.clone())
                                .unwrap_or_else(|| "<unknown spec>".to_owned()),
                            CheckedSpecRef::Std(path) => path.join("."),
                        };
                        format!("{name}{}", printer.args(&bound.args))
                    })
                    .collect::<Vec<_>>();
                if bounds.is_empty() {
                    name
                } else {
                    format!("{name} ~ {}", bounds.join(" + "))
                }
            })
            .collect::<Vec<_>>();
        let params = signature
            .params
            .iter()
            .enumerate()
            .map(|(index, ty)| {
                let ty = printer.ty(*ty);
                names
                    .get(index)
                    .map(|name| format!("{name}: {ty}"))
                    .unwrap_or(ty)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let row = signature
            .effects
            .as_ref()
            .map(|row| format!(" {}", printer.row(row)))
            .unwrap_or_default();
        format!(
            "{keyword} {name}{}({params}) -> {}{row}",
            type_params(&generics),
            printer.ty(signature.output)
        )
    }

    fn ty(&self, ty: TypeId) -> String {
        type_name(&self.snapshot.type_store, self.generics, ty, 0)
    }

    fn args(&self, args: &[TypeId]) -> String {
        type_params(&args.iter().map(|ty| self.ty(*ty)).collect::<Vec<_>>())
    }

    fn arg(&self, arg: &EffectArgRef) -> String {
        match arg {
            EffectArgRef::Type(ty) => self.ty(*ty),
            EffectArgRef::Path(path) => path.join("."),
            EffectArgRef::String(value) => format!("{value:?}"),
            EffectArgRef::Int(value) => value.clone(),
            EffectArgRef::Wildcard => "_".to_owned(),
        }
    }

    fn row(&self, row: &EffectRowRef) -> String {
        let mut effects = row
            .effects
            .iter()
            .map(|effect| {
                format!(
                    "{}{}",
                    effect.name,
                    type_params(
                        &effect
                            .args
                            .iter()
                            .map(|arg| self.arg(arg))
                            .collect::<Vec<_>>()
                    )
                )
            })
            .collect::<Vec<_>>();
        if let Some(tail) = &row.tail {
            effects.push(tail.clone());
        }
        format!("![{}]", effects.join(", "))
    }

    fn solved_row(&self, row: &etas_effects::EffectRow) -> Option<String> {
        use etas_effects::Effect;
        if row.open.is_some() {
            return None;
        }
        let registry = &self.snapshot.effect_registry;
        let effects = row
            .effects
            .iter()
            .map(|effect| {
                Some(match effect {
                    Effect::Tag(tag) => registry.tag_name(*tag)?.to_owned(),
                    Effect::Action(action) => self.action_name(action)?,
                    Effect::AppliedAction(action) => format!(
                        "{}{}",
                        self.action_name(&action.action)?,
                        type_params(
                            &action
                                .args
                                .iter()
                                .map(|arg| self.arg(arg))
                                .collect::<Vec<_>>()
                        )
                    ),
                    Effect::Applied { tag, args } => {
                        format!("{}{}", registry.tag_name(*tag)?, self.args(args))
                    }
                    Effect::Error(ty) => format!("Error<{}>", self.ty(*ty)),
                    Effect::Var(_) => return None,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(format!("![{}]", effects.join(", ")))
    }

    fn action_name(&self, action: &etas_effects::ActionRef) -> Option<String> {
        let registry = &self.snapshot.effect_registry;
        Some(format!(
            "{}.{}",
            registry.tag_name(action.tag)?,
            registry.action_name(action.tag, action.action)?
        ))
    }
}

fn type_params(params: &[String]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!("<{}>", params.join(", "))
    }
}

fn type_name(
    store: &TypeStore,
    generics: &[CallableGenericParam],
    ty: TypeId,
    depth: usize,
) -> String {
    if let Some(param) = generics.iter().find(|param| param.subject == ty) {
        return param.name.clone();
    }
    if depth >= 32 {
        return "...".to_owned();
    }
    let inner = |ty| type_name(store, generics, ty, depth + 1);
    let args = |args: &[TypeId]| {
        args.iter()
            .map(|ty| inner(*ty))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match store.get(ty) {
        Some(Type::Function(flow)) => {
            let params = args(&flow.input);
            // Function values carry their own latent effect row, not the enclosing flow's row.
            let effects = flow
                .effects
                .as_ref()
                .map(|row| {
                    let entries = row
                        .effects
                        .iter()
                        .map(|effect| {
                            let arguments = effect
                                .args
                                .iter()
                                .map(|arg| match arg {
                                    EffectArgRef::Type(ty) => inner(*ty),
                                    EffectArgRef::Path(path) => path.join("."),
                                    EffectArgRef::String(value) => format!("{value:?}"),
                                    EffectArgRef::Int(value) => value.clone(),
                                    EffectArgRef::Wildcard => "_".to_owned(),
                                })
                                .collect::<Vec<_>>();
                            format!("{}{}", effect.name, type_params(&arguments))
                        })
                        .chain(row.tail.iter().cloned())
                        .collect::<Vec<_>>();
                    format!(" ![{}]", entries.join(", "))
                })
                .unwrap_or_default();
            format!("({params}) -> {}{effects}", inner(flow.output))
        }
        Some(Type::Array(ty)) => format!("Array<{}>", inner(*ty)),
        Some(Type::List(ty)) => format!("List<{}>", inner(*ty)),
        Some(Type::Map { key, value }) => format!("Map<{}, {}>", inner(*key), inner(*value)),
        Some(Type::Set(ty)) => format!("Set<{}>", inner(*ty)),
        Some(Type::Option(ty)) => format!("Option<{}>", inner(*ty)),
        Some(Type::Result { ok, err }) => format!("Result<{}, {}>", inner(*ok), inner(*err)),
        Some(Type::Tuple(types)) => format!("({})", args(types)),
        Some(Type::Record(record)) => format!(
            "{{ {} }}",
            record
                .fields
                .iter()
                .map(|field| format!("{}: {}", field.name, inner(field.ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Some(Type::Applied {
            constructor,
            args: types,
        }) => format!("{}<{}>", inner(TypeId(constructor.0)), args(types)),
        Some(Type::Trust { wrapper, inner: ty }) => format!("{wrapper}<{}>", inner(*ty)),
        Some(Type::Refined { base, .. }) => inner(*base),
        _ => etas_types::ty::display_type(store, ty),
    }
}
