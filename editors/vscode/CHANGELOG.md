# Change Log

All notable changes to the "etas" extension will be documented in this file.

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
