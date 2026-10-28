# Compiler Blockers Found During Semantic Token Verification

Updated: 2026-10-28

These cases use Cargo-locked frontend `21c1cd88` and core `4946c9f9`.
Two compile-blocked cases remain explicitly ignored. The two resolved cases
below were rerun successfully and are now ordinary positive regressions. No IDE
highlighting fallback or diagnostic suppression is used.

## Imported Standard Array (Resolved)

```etas
module tokens;
import std.collections.Array as Seq;
import std.collections.len as count;
flow size(args: Seq<string>) -> usize { return count(args); }
flow main() -> i32 { return 0; }
```

Expected: the imported standard Array supports the standard length operation.
The former `LengthInput` mismatch at frontend `f908b216` is fixed upstream.
This now checks and highlights the imported callable with the same source.

```bash
cargo test -p etas_intel --locked --offline \
  imported_std_array_satisfies_length_input -- --nocapture
```

## Renamed Source Re-Export

```etas
// values.es
module values;
public flow square(x: i32) -> i32 { return x * x; }

// bridge.es
module bridge;
public import values.square as forwarded;

// main.es
module main;
import bridge.*;
flow main() -> i32 { return forwarded(2); }
```

Expected: the re-export retains the original flow's checked signature/effects.
Actual: `IncompleteEffectFacts` (`source import target requires checked effect
facts`). The former type errors are resolved, but effect export remains blocked. Directly
importing `values.square as forwarded` checks. The IDE must not synthesize a
callable signature or claim semantic success for the failing re-export case.

```bash
cargo test -p etas_intel --locked --offline \
  blocked_renamed_source_reexport_keeps_callable_facts -- --ignored --nocapture
```

## Metadata-Only Wildcard Effect Import

With real driver-generated dependency metadata for `remote.api.Remote` and
`remote.api.fetch`, this source fails before a checked snapshot is available:

```etas
module app;
import remote.api.*;
flow main() -> i32 ![Remote.request] {
    let body = fetch("https://example.com/status");
    return 0;
}
```

Actual: `UnresolvedName` for `Remote.request`. Explicitly importing the effect
with `import remote.api.Remote;` checks while keeping the identical public
effect contract. The metadata-only flow-classification regression uses that
explicit effect import to isolate wildcard flow bindings; this broader effect
import case remains a separate blocked test. No IDE resolution substitute is
introduced.

```bash
cargo test -p etas_intel --locked --offline \
  blocked_metadata_only_wildcard_effect_import -- --ignored --nocapture
```

## Module-Qualified Record Constructor (Resolved)

```etas
// models.es
module models;
public type Identity = { id: string };

// main.es
module main;
import models.Identity as User;
import models;
flow qualified(id: string) -> User { return models.Identity { id = id }; }
flow main() -> i32 { return 0; }
```

Expected: the qualified constructor resolves to the imported nominal type.
The former unresolved-constructor `TypeMismatch` at `f908b216` is fixed upstream.
Qualified `models.Identity { ... }` and imported alias construction now both
check and highlight through the public snapshot, without IDE resolution.

```bash
cargo test -p etas_intel --locked --offline \
  module_qualified_record_constructor -- --nocapture
```

## Selected Method Signature API Gap

```etas
module methods;
flow append(values: Array<string>, item: string) -> Array<string> {
    return values.push(item).push(item);
}
flow main() -> i32 { return 0; }
```

This source checks successfully. Checked HIR call shapes and expression types
support callable highlighting for both method names. Public `TypeFacts` now have
`std_method_calls` with checked target/signature facts for some registered
intrinsics. The IDE consumes these facts (tested with `std.compiler.with_body`),
but they are not emitted for the Array.push calls above. In addition,
`definition_target_for_expr` on a partial path such as `values.push` returns the
resolved prefix (`values`), not the method. Reusing that target for member hover
incorrectly displayed the receiver's declaration.

The IDE now rejects that receiver-as-member hover and keeps checked field-type
hover where available. Full method hover/definition requires a public per-call
selected target and instantiated callable signature, including generic/spec
bindings and effect/action contracts. No IDE method-name table, overload solver,
or synthetic signature is introduced. Unlike the two compile-blocked fixtures
above, this is an API gap after a successful check; the active Rust/LSP tests
verify highlighting and reject misleading receiver hover.
