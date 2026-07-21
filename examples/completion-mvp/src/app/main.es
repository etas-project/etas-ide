module app.main;

type Draft = string;

effect AgentRuntime {}

policy SafeReview {
    allow [AgentRuntime];
}

let defaultDraft = "outline";

agent Reviewer(input: Draft) -> Draft {}

tool normalize(input: string) -> string {
    return "normalized";
}

flow summarize(input: Draft) -> Draft {
    return input;
}

flow score(left: i32, right: i32) -> i32 {
    let total = left + right;

    // Phase 3 scope example: trigger completion below.
    // Expected candidates include left, right, and total.
    return total;
}

flow nested_scope(input: i32) -> i32 {
    let outer = input;

    if true {
        let inner = outer;

        // Phase 3 nested scope example: trigger completion below.
        // Expected candidates include input, outer, and inner.
        return inner;
    }

    // Phase 3 nested scope example: trigger completion below.
    // Expected candidates include input and outer, but not inner.
    return outer;
}

flow declaration_order(input: i32) -> i32 {
    // Phase 3 declaration-order example: trigger completion below.
    // Expected candidates include input, but not later.
    let later = input;

    // Phase 3 declaration-order example: trigger completion below.
    // Expected candidates include input and later.
    return later;
}

flow shadowing(value: i32) -> i32 {
    let outer = value;

    if true {
        let value = outer;

        // Phase 3 shadowing example: trigger completion below.
        // Expected candidates include one value candidate for the inner local,
        // plus outer. The parameter value should be shadowed here.
        return value;
    }

    return outer;
}

flow main() -> i32 {
    // Trigger completion on the blank line below.
    // Expected MVP candidates include keywords such as flow/type/agent/tool,
    // plus top-level symbols such as Draft, AgentRuntime, SafeReview,
    // defaultDraft, Reviewer, normalize, summarize, score, nested_scope,
    // declaration_order, and shadowing.

    return 0;
}
