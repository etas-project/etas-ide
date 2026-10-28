# Change Log

All notable changes to the "etas" extension will be documented in this file.

## 0.3.4 - 2026-10-28

- Align the bundled compiler, standard library, and driver with the published CLI dependency set. Agent-scoped `perform infer<T>(...)` now parses and checks without the old unresolved-action cascade.
- Adapt agent default-entry signatures, external spec bounds, generic alias constraints, and canonical effect names to the current public APIs.
- Add an Etas status bar item driven by actual server notifications: waiting, package loading, analysis, completion, and failure, with elapsed time, source count, and diagnostics.
- Show the extension, language-server, frontend, core/std, and driver identities in the tooltip. Producer identities are generated from Cargo.lock; clicking opens the log/restart menu.
- Preserve status ordering across initial synchronization and server restarts; hide the item outside Etas editors.
- Consume public selected standard-method signatures where available. Array.push still lacks a selected target fact and is not assigned a synthetic hover.
- Promote imported Array length and qualified nominal constructors from blocked cases to active regressions; retain the two remaining compiler blockers.
- Verify agent inference, out-of-agent rejection, lifecycle status, package failure/recovery, and the real VS Code status bar.

## 0.3.3 - 2026-10-27

- Highlight checked member calls such as `values.push(...)`, chained calls, and methods on grouped or literal receivers.
- Preserve receiver variables when their names collide with method names; do not treat non-call fields as methods.
- Reject method tokens for invalid calls and mismatched last-good source snapshots.
- Stop showing a partial path's receiver declaration when hovering its field or method; keep checked field-type hovers.
- Add Rust, UTF-16 LSP, and real VS Code checks for every `.push(...)` occurrence in the nomo source corpus.
- Document that full method signatures remain blocked on public selected-method target/signature facts, without adding IDE overload resolution.

## 0.3.2 - 2026-10-27

- Highlight record constructor paths such as `Identity { ... }` as types, including imported aliases and generic constructors.
- Keep named field labels, shorthand values, and nested constructor arguments in their checked categories.
- Preserve enum-member classification for named enum payload constructors.
- Show checked type hover on record constructor names.
- Add exact-position LSP and VS Code regressions for the reported `harness_checkpoint_args.es` constructor, including UTF-16 offsets and stale-source rejection.
- Record the upstream module-qualified record constructor blocker without adding an IDE resolver workaround.

## 0.3.1 - 2026-10-27

- Classify standard-library imports and aliases such as `len as count` as functions at both declaration and usage sites.
- Preserve target kinds for imported source types, specs, modules, and checked dependency metadata bindings.
- Cover grouped/wildcard imports, callable references, local shadowing, and standard enum constructors.
- Keep record field labels and generic parameter uses distinct from type names.
- Highlight effect action declarations and perform references as functions.
- Respect compiler-checked contextual keyword names such as `flow resume`, including hover, without repainting actual control keywords.
- Add Rust, stdio LSP, and real VS Code regressions for the reported `nomo/cli/edit_args.es` case.
- Record three upstream compiler blockers as explicit ignored positive tests rather than introducing IDE semantic fallbacks.

## 0.3.0 - 2026-10-26

- Replace hover's internal Debug dumps with checked Etas signatures and readable types.
- Show parameter names, generic/spec bounds, nominal identities, and nested function types.
- Preserve public effect contracts and qualified, scoped requested-action summaries.
- Resolve imported source signatures and include available standard-library descriptions.
- Return identifier-only UTF-16 hover ranges and reject mismatched last-good source facts.
- Cover hover in Rust, stdio LSP, and real VS Code extension-host regression tests.
- Include source text in the public Rust HoverResult for accurate protocol range conversion.
- Mark native VSIX packages with their target platform.

## 0.2.0 - 2026-10-26

- Cover all compiler keywords and complete numeric/character literal highlighting.
- Provide parser-backed Outline, including enum members, effect actions, and methods.
- Preserve current-buffer lexical highlighting during parse errors.
- Use identifier-only semantic ranges, UTF-16 columns, and single-line token segments.
- Publish versioned diagnostics with source text for unopened project files.
- Surface package preparation failures and reload after manifest/metadata changes.
- Update to current published compiler APIs and configure the analysis worker stack.
- Avoid recompiling the project when opening unchanged source files.
- Add TextMate, stdio LSP, and real VS Code extension-host regression suites.

## 0.1.0

- Initial release.
