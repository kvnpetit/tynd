//! Native bridge for `performance.now()`. Monotonic clock anchored at the
//! first call; return value is a double-precision milliseconds offset.

use rquickjs::{Ctx, Function, Object};
use std::sync::OnceLock;
use std::time::Instant;

fn origin() -> Instant {
    static INIT: OnceLock<Instant> = OnceLock::new();
    *INIT.get_or_init(Instant::now)
}

pub(crate) fn register<'js>(ctx: &Ctx<'js>, g: &Object<'js>) -> rquickjs::Result<()> {
    // Anchor the clock now so the first `performance.now()` call returns a
    // small value instead of the process's absolute lifetime.
    let _ = origin();
    g.set(
        "__tynd_perf_now__",
        Function::new(ctx.clone(), || -> f64 {
            let elapsed = origin().elapsed();
            (elapsed.as_secs_f64()) * 1000.0
        })?,
    )?;
    Ok(())
}
