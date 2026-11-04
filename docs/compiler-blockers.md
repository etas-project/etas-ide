# Compiler Blockers Found During Semantic Token Verification

Updated: 2026-11-04

These cases use Cargo-locked frontend `5fad86c4` (0.3.1) and core `563dedc1`.
One compile-blocked case remains explicitly ignored. The resolved cases
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

## Renamed Source Re-Export (Resolved)

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
The former `IncompleteEffectFacts` at frontend `21c1cd88` is fixed upstream.
The unchanged source now checks, preserves the callable binding, and is covered
by an active positive regression. No IDE signature/effect synthesis is used.

```bash
cargo test -p etas_intel --locked --offline \
  renamed_source_reexport_keeps_callable_facts -- --nocapture
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

## Selected Standard Method Signature (Resolved)

```etas
module methods;
flow append(values: Array<string>, item: string) -> Array<string> {
    return values.push(item).push(item);
}
flow main() -> i32 { return 0; }
```

Frontend 0.3.1 now supplies `TypeFacts.std_intrinsic_calls`, including the
selected `std.collections.array_push` callable signature. The IDE displays the
producer's generic signature, public effects, and available requested-action
row without guessing an overload or substituting the receiver's declaration.
Rust and LSP regressions assert the checked `array_push<T>` signature. Broader
source/spec method hover and definition integration remains separate work.

## Method Namespace Compatibility

The new method resolver does not turn arbitrary same-named free flows into
receiver methods (see the frontend architect's `method-resolution-design.md`).
`std.text.trim` is a free flow, so `trim(text)` is used after import; `text.trim()`
is rejected. A negative IDE regression preserves that diagnostic instead of
adding a resolver or syntax rewrite in the extension.
