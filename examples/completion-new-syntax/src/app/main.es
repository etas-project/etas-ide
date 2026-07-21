module app.main;

// ============================================================
// TOP-LEVEL KEYWORD COMPLETION
// ============================================================
// Trigger completion at a blank top-level position below:
//   Expected: alias, spec, impl, flow, type, agent, tool, ...
//   Not expected: return, if, let (expression keywords)

// ============================================================
// TYPE ALIAS COMPLETION
// ============================================================
// alias declarations in the current module

alias Title = string;

// ============================================================
// SPEC COMPLETION
// ============================================================
// spec declarations in the current module

spec ByteStream;

// ============================================================
// TYPE-POSITION VERIFICATION
// ============================================================
// Trigger completion in the parameter/return type positions below:
//   flow identity(value: /*<cursor>*/) -> /*<cursor>*/ {
//   Expected: Title (Type, current-module alias)
//   Expected: ByteStream (Spec, current-module spec)
//   Expected: i32, string, bool (primitive types)
//   Not expected: identity, main (flow/expression-only symbols)

flow identity(value: Title) -> Title {
    return value;
}

// ============================================================
// EXPRESSION-POSITION VERIFICATION
// ============================================================
// Trigger completion inside `return /*<cursor>*/` below:
//   Expected: value (Variable, parameter)
//   Expected: identity (Flow, current-module flow)
//   Expected: main (Flow, current-module flow)
//   Expected: expression keywords (if, let, return, ...)
//   Not expected: Title, ByteStream (type/spec in expression)
//   Not expected: alias, spec, impl (top-level keywords)

// ============================================================
// IMPL KEYWORD AT TOP LEVEL
// ============================================================
// Trigger completion at a blank top-level position below:
//   Expected: impl among all top-level keywords

impl ByteStream for string {

}

flow main() -> i32 {
    let v = identity("hello");
    return 0;
}
