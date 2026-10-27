# Compiler Blockers Found During Semantic Token Verification

Updated: 2026-10-27

These cases use the Cargo-locked frontend revision `f908b216` and core revision
`56c50906`. They are not IDE highlighting fallbacks. Positive expected behavior
is preserved in explicitly ignored tests; running the tests below currently
fails before a checked semantic snapshot can be produced.

## Imported Standard Array

```etas
module tokens;
import std.collections.Array as Seq;
import std.collections.len as count;
flow size(args: Seq<string>) -> usize { return count(args); }
flow main() -> i32 { return 0; }
```

Expected: the imported standard Array supports the standard length operation.
Actual: `TypeMismatch`, reporting that `std.collections.Array<string>` does not
match `LengthInput`. The same operation on the prelude `Array<string>` checks.
The IDE must not rewrite the imported type to a prelude type or bypass checking.

```bash
cargo test -p etas_intel --locked --offline \
  blocked_imported_std_array_satisfies_length_input -- --ignored --nocapture
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
Actual: `UnknownType` / `NonCallableCallee` and `IncompleteEffectFacts`. Directly
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
