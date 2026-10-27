# Etas IDE Status

Version: 0.3.3

Updated: 2026-10-27

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
- Checked hover signatures for flows/tools/agents, parameter names and types,
  generic/spec bounds, nominal types, nested function values, public effect
  contracts, and qualified requested-action scopes. Source imports use their
  resolved declarations; std symbols include available registry descriptions.
- Identifier/token-only UTF-16 hover ranges. Mismatched last-good source facts
  are not attached to edited identifiers. Open requested-action rows are marked
  unresolved rather than presented as empty/pure.
- Metadata-aware semantic token classification for standard/source/package
  imports, aliases, grouped/wildcard imports, callable references, and local
  shadowing. Module/type imports are not guessed to be functions. Record field
  labels, generic parameter uses, enum constructors, and action references keep
  their own token categories.
- Checked record constructor paths (including imported aliases and generics),
  named field initializers, shorthand values, nested records, and named enum
  payloads. Nominal record constructor names also expose their checked type hover.
- Checked member calls for value paths, chained calls, grouped/literal receivers,
  and repeated receiver/method names. Method-name tokens exclude the receiver and
  arguments. Invalid calls do not become guessed functions, and partial member
  hover no longer presents the receiver's declaration as the member's signature.

## Verification

The 0.3.3 development verification includes:

- Rust workspace regression tests: 66 passed, 4 compiler-blocked positive tests
  explicitly ignored. Metadata-only dependency import coverage uses real
  driver-generated package metadata.
- Strict workspace Clippy and formatting checks.
- Extension TypeScript compile and esbuild bundle.
- TextMate tests with VS Code's tokenizer and Oniguruma; the keyword list is
  read from the exact Cargo-resolved compiler revision, not a copied list.
- Real stdio LSP tests for initialization, Outline, lexical and semantic
  ranges, parse errors, versioned diagnostics, UTF-16 incremental edits,
  recovery, package preparation failure/reload, checked hover content/imports,
  scoped actions, and stale-source hover rejection. All 71 real-project files
  also passed checked hover assertions with zero error diagnostics.
- Real VS Code extension-host tests against `~/Projects/nomo`: 71 source
  files with Outline, semantic providers, and 71 readable flow hovers, plus unsaved
  parse-error/recovery and Unicode hover-range tests in a temporary document.
- TextMate verification over the same 71 files: 6,300 keyword occurrences,
  including contextual keyword names. The reported `edit_args.es` import alias
  and its calls are asserted as functions in both LSP and VS Code tests.
- The exact `Identity { id = id, workspace = workspace, ... }` occurrence in
  `nomo/cli/harness_checkpoint_args.es` is checked at its source position in
  both LSP and VS Code provider tests, not just by matching any `Identity` token.
- All `.push(...)` source occurrences in that corpus are checked at their exact
  positions through both the LSP and the real VS Code semantic provider. Dedicated
  member-call tests cover Unicode, chains, literals, grouping, name collisions,
  field/method distinction, invalid calls, stale-source rejection, and recovery.

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

Four positive cases remain blocked by the linked compiler, with executable
ignored tests and exact reproducers in [Compiler blockers](compiler-blockers.md):
imported std Array length support, renamed source re-export callable facts, and
metadata-only wildcard effect imports, and module-qualified nominal record
constructors. The IDE does not synthesize semantic
success for these cases or relax their expected behavior.

Full member-method signature hover also needs public selected-method facts from
the frontend. Checked call shape/type facts are enough to classify method names,
but not to reconstruct a chosen overload's definition, parameter names, generic
bindings, or effect contract. The IDE does not guess an overload or substitute
the receiver's declaration; see the API gap in [Compiler blockers](compiler-blockers.md).

Workspace symbols, references, code actions, formatting, cancellation of an
in-flight compiler query, and simultaneous per-package sessions are not yet
implemented. The current service switches its active semantic project when a
document in another package is activated. It does not claim simultaneous
multi-root semantic sessions.

Definition may retain last-good semantic facts during invalid edits; hover
rejects mismatched source text, while Outline and tokens use the current buffer.
A broader current-source semantic
recovery model belongs in the frontend API, not a second IDE type checker.

The bundled server is native. Build/install a VSIX for the target operating
system and architecture; do not ship a binary built for a different platform.
