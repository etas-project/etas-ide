# Etas IDE Status

Version: 0.2.0

Updated: 2026-10-26

## Implemented

- VS Code TextMate grammar covering every keyword in the linked compiler's
  `Keyword::from_ident` table, declarations, annotations, primitive types,
  numeric/character/string literals, comments, and operators.
- LSP incremental document sync, versioned parse/name/type/effect diagnostics,
  hover, completion, definition, document symbols, and full semantic tokens.
- Parser-backed Outline for modules, types, aliases, enums/variants,
  specs/methods, impl methods, effects/actions, tools, agents, and flows.
- UTF-16 ranges and semantic-token deltas; multiline tokens are split into
  non-overlapping single-line segments. Broad HIR origins cannot paint whole
  expressions or declarations as a single identifier.
- Current-buffer lexical tokens and Outline during incomplete edits. Checked
  semantic classifications are used only when their source text matches the
  buffer.
- Recursive project discovery and package dependency overlays through public
  frontend/driver/package APIs. No compiler or interpreter implementation is
  owned by this repository.
- Explicit package preparation failure diagnostics and reload on manifest,
  lockfile, or package-index changes. Invalid package setup is not reported as
  a successful standalone analysis.
- Unchanged disk-backed document opens reuse the checked project. Hover and
  completion do not rebuild an already-prepared package on every request.
- Dedicated 8 MiB compiler analysis worker stack, stdio-safe logging, bundled
  native LSP binary, and VSIX packaging.

## Verification

The 0.2.0 development verification includes:

- `cargo test --workspace --offline`: 42 tests passed.
- Extension TypeScript compile and esbuild bundle.
- TextMate tests with VS Code's tokenizer and Oniguruma; the keyword list is
  read from the exact Cargo-resolved compiler revision, not a copied list.
- Real stdio LSP tests for initialization, Outline, lexical and semantic
  ranges, parse errors, versioned diagnostics, UTF-16 incremental edits,
  recovery, and package preparation failure/reload.
- Real VS Code extension-host tests against `~/Projects/nomo`: 52 source
  files with Outline and semantic providers, plus unsaved parse-error/recovery
  and Unicode-position tests in a temporary document.
- TextMate verification over the same 52 files: 4,565 keyword occurrences.

The project tests read source files but do not run model, network, tool, or
runtime actions. Package loading may acquire the package manager's local
metadata lock.

## Reproduce

```bash
cargo build -p etas_lsp --locked
cargo test --workspace --locked --offline
cd editors/vscode
npm ci
npm run compile
ETAS_E2E_PROJECT="$HOME/Projects/nomo" npm test
ETAS_E2E_PROJECT="$HOME/Projects/nomo" npm run test:lsp
npm run package:vsix
```

For the real editor gate, launch a local VS Code executable with:

- `--extensionDevelopmentPath=<repo>/editors/vscode`
- `--extensionTestsPath=<repo>/editors/vscode/test/vscode-host.cjs`
- isolated `--user-data-dir` and `--extensions-dir`
- `ETAS_E2E_PROJECT` pointing at the target project
- optional `ETAS_LSP_BIN` for a development binary; otherwise the extension
  uses its bundled server
- optional `ETAS_E2E_RESULT` for a JSON success/failure report

## Remaining Work

Workspace symbols, references, code actions, formatting, cancellation of an
in-flight compiler query, and simultaneous per-package sessions are not yet
implemented. The current service switches its active semantic project when a
document in another package is activated. It does not claim simultaneous
multi-root semantic sessions.

Hover/definition may retain last-good semantic facts during invalid edits;
Outline and tokens use the current buffer. A broader current-source semantic
recovery model belongs in the frontend API, not a second IDE type checker.

The bundled server is native. Build/install a VSIX for the target operating
system and architecture; do not ship a binary built for a different platform.
