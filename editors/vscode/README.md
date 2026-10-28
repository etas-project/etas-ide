# Etas

The status bar reports the active Etas analysis project, package loading and
check progress, last check duration, and diagnostic counts. Hover it to inspect
the bundled compiler revisions and server path; click it to open the language
server log or restart the server. Compiler revisions come from the extension's
build, not from a separate `etas` executable on PATH.

Etas language support for Visual Studio Code.

## Features

- **Syntax Highlighting** - All compiler keywords, declaration names, annotations, primitive types, literals, comments, and operators.
- **Diagnostics** - Inline parse/type/effect errors and warnings, refreshed as you type (after a short debounce).
- **Hover** - Type and symbol info under the cursor.
- **Go to Definition** - Jump to definitions, across files.
- **Completions** - Context-aware completions for keywords, symbols, imports, types.
- **Semantic Highlighting** - Semantic colors for functions, types, params, variables.
- **Outline** - Current parser declarations, spec/impl methods, effect actions, and enum members.

All positions use UTF-16. Highlighting and Outline remain available during
incomplete edits; package setup failures are reported explicitly in Problems.
Changing `etas.toml`, `etas.lock`, or package-index metadata reloads analysis.

> Not yet supported: find references, workspace symbols, code actions.

## Requirements

No additional dependencies.

## Installation

### From VSIX

1. Download the `.vsix` package from the [releases page](https://github.com/etas-project/etas-ide/releases)
2. Install in VS Code.

### From source

1. Clone the repository
2. Open `editors/vscode` in VS Code
3. Press `F5` to launch a new Extension Development Host window

## Release Notes

See CHANGELOG.md.
