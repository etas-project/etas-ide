# Etas IDE

VS Code support for Etas: highlighting, diagnostics, completion, hover, and
go to definition.

The extension runs a language server that reuses the Etas frontend for project
loading and type/effect diagnostics. Editor feedback and CLI checking therefore
share the same language implementation rather than separate checking rules.

Experimental. Requires Rust 1.85+, Node.js 22+, VS Code 1.100+, and the
`code` command. Building the bundled server also requires a native build
toolchain with `strip`.

## Quickstart

```sh
git clone https://github.com/etas-project/etas-ide.git
cd etas-ide/editors/vscode
npm ci
export ETAS_STD_SYSROOT="$(mktemp -d)"
npm run build-std
npm run package:vsix
code --install-extension etas-*.vsix
```

Open an `.es` file and use Go to Definition to navigate into the bundled
Etas standard library. Set `etas.stdProvider` to `legacy` for a project
that needs the compatibility provider.

For a package project, open the folder containing `etas.toml` so the server can
load the project and its imports. Build and run programs with the
[Etas CLI](https://github.com/etas-project/etas#quickstart); the extension provides
editing support, not a second interpreter.

`build-std` builds the SDK from the revisions in `Cargo.lock`. To reuse an
existing matching SDK, set `ETAS_STD_SYSROOT` to the directory containing
`std/manifest.json`. `package:vsix` bundles that SDK and the language server.

## Documentation

- [Extension settings and development](editors/vscode/README.md)
- [Feature status](docs/STATUS-p0.md)

[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)
