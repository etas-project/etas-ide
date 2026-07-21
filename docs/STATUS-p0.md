# Etas IDE — Status & Roadmap

## Current Status

### What works

| Feature | Status |
|---|---|
| **Server scaffolding** (init/shutdown/capabilities) | Done |
| **Document sync** (didOpen/didChange/didClose) | Done — incremental sync |
| **Diagnostics** (parse errors, type errors) | Done — published on every change |
| **Hover** (type info on expressions) | Done — type from TypeFacts |
| **VSCode extension** | Done — activates on `.es`, spawns LSP over stdio |
| **Bundled server binary** | Done — release binary in VSIX at `server/etas-lsp` |
| **CI packaging** | Done — `package-vsix.yml` builds binary + bundles extension + creates release |

### What's missing (P1/P2)

| Feature | Priority |
|---|---|
| Debounce (150ms delay before re-check) | P1 |
| Multi-file project support (walk `src/` for `.es` files) | P1 |
| Go-to-definition | P1 |
| Completions (keywords + symbols in scope) | P2 |
| Document symbols / workspace symbols | P2 |
| Semantic tokens | P2 |
| References | P2 |
| Code actions (from diagnostic suggestions) | P2 |
| Proper type display (not `{:?}` format) | P2 |

---

## Architecture

```
VSCode Extension (TypeScript, bundled with esbuild)
  │ stdio spawn
  ▼
etas_lsp (tower-lsp 0.20, Rust)
  │ channel (tokio::mpsc)
  ▼
etas_intel (std::thread service, Rust)
  │ FrontendSession
  ▼
etas_frontend (compiler pipeline: parse → lower → resolve → check)
```

**Key design decisions:**
- `LanguageService` runs on a dedicated `std::thread` because `FrontendSession` is `!Send`
- Communication via `tokio::sync::mpsc::unbounded_channel` with `Command` enum
- `tower-lsp` 0.20 with `vscode-languageclient` 8.x
- Extension bundled with esbuild (755KB), no `node_modules` in VSIX
- Server binary (6.9MB stripped release) bundled in VSIX at `server/etas-lsp`
- Logs go to the platform temporary directory as `etas-lsp-<pid>.log`; `ETAS_LSP_LOG` overrides the path

---

## File map

```
etas-ide/
├── Cargo.toml                          # workspace: etas_intel + etas_lsp
├── .github/workflows/package-vsix.yml  # CI: checkout all repos, build binary + bundle + VSIX
├── docs/architect/
│   ├── repository-boundary.md          # architecture rules for ide repo
│   ├── lsp-p0-implementation-plan.md   # implementation plan
│   └── lsp-p0-results.md               # (outdated) build notes
├── crates/
│   ├── etas_intel/src/
│   │   ├── lib.rs                      # exports: spawn_service, Command, HoverResult, LspPosition/Types
│   │   ├── service.rs                  # LanguageService (thread), Command enum, position conversion
│   │   └── types/
│   │       └── diagnostic.rs           # IntelDiagnostic, IntelSeverity
│   └── etas_lsp/src/
│       ├── main.rs                     # binary entry: file-based tracing, tower-lsp Server
│       ├── server.rs                   # Backend (LanguageServer trait), publish diagnostics, hover
│       └── conversions.rs              # IntelDiagnostic → lsp_types::Diagnostic (byte→line/col)
└── editors/vscode/
    ├── package.json                    # activationEvents, scripts (build, build-server, compile)
    ├── tsconfig.json                   # commonjs, skipLibCheck, strict
    ├── .vscodeignore                   # excludes src/, tsconfig, node_modules; keeps out/, dist/, server/
    ├── .vscode/launch.json             # F5 debug configs
    ├── .vscode/tasks.json              # npm build/compile tasks
    ├── src/extension.ts                # activate: resolveServerPath, LanguageClient setup
    ├── server/etas-lsp                # (generated) release binary
    ├── dist/extension.js               # (generated) esbuild bundle
    └── out/extension.js                # (generated) tsc output (for dev only)
```

---

## Build & run

### Quick build

```bash
# In one command
cd etas-ide/editors/vscode && vsce package
# vscode:prepublish runs: esbuild bundle + cargo build --release + copy binary
```

### Individual steps

```bash
# Server binary
cd etas-ide && cargo build -p etas_lsp --release

# Extension bundle
cd etas-ide/editors/vscode
npm install
npm run build          # esbuild

# Bundle binary into extension
npm run build-server   # cargo build --release + cp to server/

# Package VSIX
npx vsce package --allow-missing-repository
```

### Smoke test

```bash
build_frame() { local len=$(echo -n "$1"|wc -c); printf "Content-Length: %d\r\n\r\n%s" "$len" "$1"; }
{
  build_frame '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"capabilities":{}}}'
  sleep 0.1
} | timeut 3 target/release/etas-lsp
# Expected: hoverProvider: true, textDocumentSync: 2
```

### Install in VSCode

```
Ctrl+Shift+P → "Extensions: Install from VSIX..."
→ select: editors/vscode/etas-0.1.0.vsix
→ Reload Window
→ Open any .es file
```

### Debug extension

```
Open etas-ide/editors/vscode/ in VSCode
F5 → "Launch Extension"
→ Opens Extension Dev Host with extension loaded
```

### View logs

```bash
ETAS_LSP_LOG="$PWD/etas-lsp.log" code .
tail -f etas-lsp.log
```

---

## CI (GitHub Actions)

`.github/workflows/package-vsix.yml` does:

1. Checkout `etas-ide`
2. Setup Rust stable + Node 24
3. `npm ci` using the committed npm lockfile
4. `npm run package:vsix`:
   - `cargo build --release` + copy the LSP binary to `server/`
   - `vsce package` triggers `vscode:prepublish` and the esbuild bundle
5. Create GitHub Release (dev prerelease on push, proper release on tag)

---

## Cross-repo dependency setup

This repo uses Git dependencies for cross-repository crates:

```toml
# etas-ide/Cargo.toml
etas_core = { git = "https://github.com/etas-project/etas-core.git" }
etas_frontend = { git = "https://github.com/etas-project/etas-frontend.git" }
etas_hir = { git = "https://github.com/etas-project/etas-frontend.git" }
etas_types = { git = "https://github.com/etas-project/etas-frontend.git" }
```

Cargo resolves these dependencies from the public repositories in CI and normal
standalone clones. Local overrides, when needed, belong in an ignored
`.cargo/config.toml`, not in committed manifests.

---

## Known quirks & future work

1. **Protocol-safe logging** — logs stay off stdout and use a per-process temporary file unless `ETAS_LSP_LOG` is set.
2. **No debounce** — every keystroke triggers a full check. Add 150ms timer.
3. **Single-file only** — multi-file projects need source discovery (walk `src/`).
4. **Type display raw** — hover shows internal type representation (`Primitive(Int)`). Needs friendly display.
5. **Thread-based service** — adds ~1ms latency per query. Could switch to `!Send` workaround with tower-lsp local task spawn.
6. **Release binary is 6.9MB** — could be smaller with LTO, or if `etas-core` symbol count is reduced.
7. **`tower-lsp` 0.20 → `lsp-types` 0.94** — URLs use `Url::parse()` not `Uri::from_str()`. Don't add lsp-types 0.97 separately to the workspace.
8. **VSIX ~2.75MB** — fine for now. The extension bundle (755KB) + stripped server binary (6.9MB raw, 2MB compressed).
