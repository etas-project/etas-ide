module app.main;

import app.models.UserId;
import app.utils.helper;
import app.math.*;

let helperName = "local-shadow";

flow imported_flow() -> i32 {
    // Phase 3 imported symbol example: trigger completion below.
    // Expected candidates include helper from app.utils.helper and add from app.math.*.
    // hidden should not appear because it is not public.
    return helper() + add(1, 2);
}

flow imported_type(id: UserId) -> i32 {
    // Phase 3 imported type example: trigger completion in the annotation above.
    // Expected type-context candidates include UserId from app.models.UserId.
    return id;
}

flow local_shadows_import() -> i32 {
    let helper = 2;

    // Phase 3 shadowing example: trigger completion below.
    // Expected candidates include one helper, the local variable, not the imported flow.
    return helper;
}

flow parameter_shadows_top_level(helperName: string) -> string {
    // Phase 3 shadowing example: trigger completion below.
    // Expected candidates include one helperName, the parameter, not the top-level let.
    return helperName;
}

flow main() -> i32 {
    let from_import = imported_flow();
    let from_local = local_shadows_import();
    return from_import + from_local;
}
