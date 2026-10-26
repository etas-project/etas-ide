# Etas IDE

Language intelligence, LSP transport, and editor packaging for Etas.

This repository reuses the Etas driver/frontend for project loading and static
semantics. It does not implement a second parser, resolver, type checker, or
effect checker.

## Feature Status

| Feature | Current state |
|---|---|
| `.es` syntax highlighting | Available |
| Incremental syntax/name/type/effect diagnostics | Available |
| Hover | Available |
| Go to definition, including import targets | Available |
| Contextual completion | Available |
| Semantic tokens | Available |
| VS Code extension with bundled server | Available |
| CI build and VSIX packaging | Available |
| Find references | Not implemented |
| Document symbols / Outline | Available, from the current frontend AST |
| Workspace symbols | Not implemented |

The LSP server advertises only implemented capabilities. Pending features must
not be exposed as empty successful responses.

## Architecture

```text
VS Code extension (TypeScript + esbuild)
                 |
                 | stdio / LSP
                 v
             etas_lsp
      protocol, lifecycle, conversion
                 |
                 | typed commands + replies
                 v
            etas_intel
  documents, overlays, snapshots, queries
          /                 \
         v                   v
   etas_driver         FrontendSession
 project/environment    parse -> checked HIR
```

### `etas_lsp`

`etas_lsp` is a thin protocol adapter. It owns initialization, capability
negotiation, document notifications, request cancellation/lifecycle, LSP type
conversion, diagnostic publication, semantic-token refresh, and stdio-safe
logging. It must not inspect HIR or implement semantic fallbacks.

### `etas_intel`

`etas_intel` is the editor-facing language service. It owns open documents,
versions, overlays, debounce/coalescing, project sessions, latest diagnostics,
last-good semantic snapshot policy, and semantic queries. It consumes
`ProjectSemanticSnapshot` rather than rebuilding symbols/types from syntax.

The service runs on a dedicated `std::thread` because the current
`FrontendSession` is not `Send`. Tokio channels and oneshot replies bridge the
async LSP server to that service thread.

### Frontend reuse

The frontend owns compilation revisions, invalidation, checked HIR, and
optional snapshot construction. The IDE owns document and snapshot lifecycle.
When an edit has blocking static errors, the IDE publishes the new diagnostics
but retains the most recent valid semantic snapshot for queries where that is
safe. Semantic answers can therefore refer to last-good state until the source
becomes valid again; clients must not mistake that state for a successful
current compilation.

Outline and lexical tokens always use the current buffer through the compiler's
public parse/token API. Semantic classifications are applied only when the
checked source matches that buffer, and only to identifier tokens. All LSP
ranges and token deltas use UTF-16, including multiline comments/strings.
Package preparation errors are published explicitly; they do not silently turn
an invalid package into a checked standalone file.

## Repository Layout

```text
crates/
  etas_intel/       project service, commands, diagnostics, locations,
                    completion, semantic tokens, and semantic queries
  etas_lsp/         tower-lsp server, handlers, and protocol conversions
editors/
  vscode/           extension activation, grammar, language configuration,
                    packaging, and bundled server location
docs/
  architect/        repository boundary
```

Committed cross-repository dependencies use Git identities. Local sibling
development should use Cargo patch configuration rather than changing these
dependencies to committed machine-relative paths.

## Build and Test

Requirements:

- Rust `1.85` or newer;
- Node.js and npm for the VS Code extension;
- compatible Etas core, frontend, and driver/package revisions.

Build and verify the Rust workspace:

```bash
cargo build -p etas_lsp --release
cargo test --workspace --offline
cargo fmt --all -- --check
cargo clippy --workspace --offline -- -D warnings
```

Build the extension:

```bash
cd editors/vscode
npm install
npm run compile
npm test
npm run test:lsp
npm run build
```

Build the server, copy it into the extension, and package a VSIX:

```bash
npm run package:vsix
code --install-extension etas-*.vsix
```

For local development, `etas.lsp.path` may point to an explicitly built
`etas-lsp` binary. LSP protocol bytes use stdout; diagnostic logging must stay
off stdout. By default, the server writes to the platform temporary directory
as `etas-lsp-<pid>.log`; set `ETAS_LSP_LOG` to choose an explicit path.

## Verification Expectations

Changes to semantic features should include tests at both boundaries:

- `etas_intel` tests for project/edit/snapshot/query behavior;
- `etas_lsp` tests for capabilities and LSP conversion;
- extension compile/build checks for packaging changes;
- regressions for broken edits, stale semantic state, multi-file imports, and
  UTF-16 position conversion.

## Documents

The real-project tests accept `ETAS_E2E_PROJECT`, for example:

```bash
cd editors/vscode
ETAS_E2E_PROJECT="$HOME/Projects/nomo" npm test
ETAS_E2E_PROJECT="$HOME/Projects/nomo" npm run test:lsp
```

`test/vscode-host.cjs` is also a VS Code extension-host suite. Launch VS Code
with `--extensionDevelopmentPath` pointing at `editors/vscode` and
`--extensionTestsPath` pointing at that file. Set `ETAS_E2E_PROJECT` to the
project and optionally `ETAS_LSP_BIN` to a development server. Use separate
`--user-data-dir` and `--extensions-dir` directories. The suite opens real
project files read-only and tests unsaved error/recovery edits in a temporary
document. `ETAS_E2E_RESULT` optionally selects a JSON result file.

- [Current IDE status](docs/STATUS-p0.md)
- [Repository boundary](docs/architect/repository-boundary.md)

## License

Etas IDE is distributed under the terms of both the
[MIT License](LICENSE-MIT) and the
[Apache License (Version 2.0)](LICENSE-APACHE). You may choose either license.
