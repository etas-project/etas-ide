# Etas IDE Repository Boundary

## Responsibility

`etas-ide` owns:

- LSP protocol server;
- editor-facing diagnostics, hover, completion, definition, symbols, and
  semantic tokens;
- virtual file snapshots and incremental query caches;
- editor graph previews;
- editor packaging.

## Crate Boundary

`etas-ide` contains two active Phase 1 crates with deliberately different
responsibilities:

```text
etas_lsp
  LSP protocol adapter and executable server

etas_intel
  protocol-neutral language intelligence service
```

They must not both implement editor features. `etas_lsp` owns transport and
protocol adaptation. `etas_intel` owns editor-oriented queries over checked
frontend state.

### `etas_lsp`

`etas_lsp` is the Language Server Protocol boundary.

It owns:

- `etas-lsp` binary entry point;
- JSON-RPC / LSP server lifecycle;
- `initialize`, `initialized`, `shutdown`, and `exit`;
- capability negotiation;
- text-document synchronization handlers such as `didOpen`, `didChange`,
  `didSave`, and `didClose`;
- request cancellation and request id handling;
- LSP request handlers for diagnostics, hover, completion, definition,
  document symbols, workspace symbols, references, semantic tokens, and code
  actions;
- conversion between protocol-neutral `etas_intel` values and `lsp_types`;
- publishing diagnostics to the editor.

It must not own:

- parser, lowering, type, effect, import, or project semantics;
- completion ranking rules that depend on Etas semantic facts;
- hover content construction from HIR/type/effect data;
- source snapshot caches beyond the state needed to route LSP notifications;
- workspace/project query caches;
- interpreter, runtime, host, model, tool, memory, sandbox, or authority logic.

Recommended internal layout:

```text
crates/etas_lsp/
  src/
    main.rs
    server.rs
    handlers.rs
    conversions.rs
    diagnostics.rs
    cancellation.rs
```

The `etas_lsp` handler shape should stay thin:

```text
LSP request
  -> decode params
  -> call etas_intel::LanguageService
  -> convert result to lsp_types
  -> return/publish response
```

### `etas_intel`

`etas_intel` is the protocol-neutral IDE language service.

It owns:

- workspace state;
- virtual file system and in-memory document snapshots;
- source versioning;
- project input construction for `etas_frontend`;
- long-lived `etas_frontend::FrontendSession` ownership per workspace/project;
- `ProjectSemanticSnapshot` and `ProjectSemanticDelta` consumption;
- secondary editor indexes derived from frontend snapshots;
- diagnostic queries;
- hover queries;
- completion queries;
- go-to-definition and reference queries;
- document symbols and workspace symbols;
- semantic token classification;
- editor graph preview data;
- stable protocol-neutral result structs used by LSP and future non-LSP
  editor integrations.

It must not own:

- LSP transport or JSON-RPC server code;
- `lsp_types` as part of its public API;
- editor-specific client capability negotiation;
- terminal or CLI rendering;
- parser, HIR, type, or effect semantics beyond calling `etas_frontend` public
  APIs and projecting the returned facts for editor use;
- runtime execution, model calls, tool calls, memory backends, sandboxing, or
  authority.

Recommended internal layout:

```text
crates/etas_intel/
  src/
    lib.rs
    service.rs
    workspace/
      mod.rs
      document.rs
      vfs.rs
      source_map.rs
    project/
      mod.rs
      input.rs
      session.rs
      indexes.rs
      diagnostics.rs
    queries/
      mod.rs
      hover.rs
      completion.rs
      definition.rs
      references.rs
      symbols.rs
      semantic_tokens.rs
    preview/
      mod.rs
      graph.rs
    types/
      mod.rs
      diagnostic.rs
      completion.rs
      hover.rs
      location.rs
```

The public facade should be centered on a language service:

```rust
pub struct LanguageService;

impl LanguageService {
    pub fn open_document(&mut self, params: OpenDocumentParams) -> IntelResult<()>;
    pub fn change_document(&mut self, params: ChangeDocumentParams) -> IntelResult<()>;
    pub fn close_document(&mut self, uri: DocumentUri) -> IntelResult<()>;

    pub fn diagnostics(&mut self, uri: DocumentUri) -> IntelResult<Vec<IntelDiagnostic>>;
    pub fn hover(&mut self, params: HoverParams) -> IntelResult<Option<HoverInfo>>;
    pub fn completion(&mut self, params: CompletionParams) -> IntelResult<Vec<CompletionItem>>;
    pub fn definition(&mut self, params: DefinitionParams) -> IntelResult<Vec<Location>>;
    pub fn document_symbols(&mut self, uri: DocumentUri) -> IntelResult<Vec<DocumentSymbol>>;
    pub fn semantic_tokens(&mut self, uri: DocumentUri) -> IntelResult<SemanticTokenSet>;
}
```

The exact names may evolve, but the public API must remain protocol-neutral.

### Frontend Facts Dependency

`etas_frontend` is the semantic source of truth for IDE intelligence.
`etas_intel` must reuse frontend outputs instead of reimplementing parser,
lowering, import resolution, name resolution, type checking, effect checking, or
standard-library semantics.

The intended dependency is:

```text
etas_intel
  -> build ProjectInput from workspace documents
  -> create and own etas_frontend::FrontendSession
  -> apply_changes from VFS/open-buffer events
  -> check with CheckMode::Incremental
  -> consume ProjectSemanticSnapshot / ProjectSemanticDelta
  -> update secondary editor indexes
  -> answer editor queries from current frontend facts
```

`etas_intel` may read and index frontend-owned facts such as:

- source files and source ids;
- parse diagnostics and source spans;
- module index and module parts;
- import graph and resolved imports;
- HIR item/body/source mappings exposed by `ProjectSemanticSnapshot`;
- resolved paths and definition targets;
- symbol tables and visibility/export facts;
- type store, item signatures, expression types, and symbol type facts;
- effect summaries, requirement facts, and interpreter-support facts;
- standard-library declaration metadata made available through frontend output.

These facts should be treated as the current checked semantic snapshot owned by
the frontend session. `etas_intel` may build secondary editor indexes over them
for fast lookup, but those indexes must not become independent semantic
authorities. Disk cache may accelerate cold start, but hover, completion,
definition, references, semantic tokens, and diagnostics must come from the
current live session view.

Query ownership:

```text
diagnostics
  frontend diagnostics -> etas_intel filtering/grouping -> LSP diagnostics

hover
  frontend symbol/type/effect facts -> etas_intel HoverInfo

completion
  frontend module/import/std/scope facts -> etas_intel ranked CompletionItem

definition/references
  frontend resolved paths + source maps -> etas_intel Location results

semantic tokens
  syntax shape + frontend symbol/type facts -> etas_intel SemanticTokenSet
```

`etas_intel` may add editor-specific ranking, filtering, snippet construction,
and display formatting. It must not decide whether a program is semantically
valid independently from `etas_frontend`.

### Dependency Direction

Allowed crate dependencies inside `etas-ide`:

```text
etas_lsp -> etas_intel
etas_lsp -> etas_core
etas_lsp -> lsp_types / LSP server library

etas_intel -> etas_core
etas_intel -> etas_frontend
etas_intel -> etas_optimizing      # optional preview-only APIs
```

Forbidden crate dependencies:

```text
etas_intel -> etas_lsp
etas_intel -> lsp_types in public API
etas_lsp -> etas_frontend directly for semantic features
etas_lsp -> etas_syntax directly for semantic features
etas_lsp -> etas_hir directly for semantic features
etas_lsp -> etas_types directly for semantic features
etas_lsp -> etas_effects directly for semantic features
etas_lsp -> etas_interpreter
etas_intel -> etas_interpreter
etas-ide  -> etas_runtime
```

`etas_lsp` may depend on `etas_core` for shared diagnostics/source/span ids
when converting outputs, but it should obtain semantic answers from
`etas_intel`, not by reconstructing frontend queries locally.

### Request Flow

Diagnostics:

```text
editor didOpen/didChange
  -> etas_lsp handler
  -> etas_intel::LanguageService updates document snapshot
  -> etas_intel updates ProjectInput overlays
  -> FrontendSession.apply_changes
  -> FrontendSession.check(CheckMode::Incremental)
  -> etas_intel consumes ProjectSemanticDelta and current snapshot
  -> etas_intel maps replaced diagnostics
  -> etas_lsp converts diagnostics to LSP and publishes them
```

Hover:

```text
textDocument/hover
  -> etas_lsp decode position
  -> etas_intel hover query
  -> etas_intel consults cached checked frontend facts
  -> protocol-neutral HoverInfo
  -> etas_lsp converts to LSP Hover
```

Completion:

```text
textDocument/completion
  -> etas_lsp decode trigger context
  -> etas_intel completion query
  -> workspace/source/std/module/type/effect facts
  -> protocol-neutral CompletionItem list
  -> etas_lsp converts to LSP CompletionResponse
```

Go to definition:

```text
textDocument/definition
  -> etas_lsp decode position
  -> etas_intel definition query
  -> frontend source maps and resolved paths
  -> protocol-neutral Location list
  -> etas_lsp converts to LSP LocationLink/Location
```

### Naming Rule

`intel` means "language intelligence", not "IDE frontend". It should not become
an editor protocol server. If a future non-LSP integration is added, it should
reuse `etas_intel` directly and provide its own protocol adapter next to
`etas_lsp`.

## Forbidden

This repository must not own:

- parser, HIR, type, or effect semantics;
- runtime authority;
- model/tool/memory/sandbox execution;
- FIR optimization.

## Dependency Rule

Allowed dependencies:

```text
etas-ide -> etas-core
etas-ide -> etas-frontend
etas-ide -> etas-optimizing      # optional preview-only APIs
```

Forbidden dependency:

```text
etas-ide -> etas-runtime
```
