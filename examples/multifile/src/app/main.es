module app.main;

import app.utils.*;
import app.math.{square};

flow main() -> i32 {
    // cursor on add -> jump to src/utils.es
    let x = add(2, 3);

    // cursor on multiply -> jump to src/utils.es
    let y = multiply(x, 4);

    let z = subtract(y, 5);

    let w = square(z);

    return w;
}
