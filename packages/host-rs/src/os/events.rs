//! Process-wide OS event emitter. Background threads emit without holding the
//! `EventLoopProxy`. `app.rs` wires `set_emitter` once at startup.

use serde_json::Value;
use std::sync::OnceLock;

pub type EmitFn = Box<dyn Fn(&str, &Value) + Send + Sync + 'static>;

static EMITTER: OnceLock<EmitFn> = OnceLock::new();

pub fn set_emitter(f: EmitFn) {
    let _ = EMITTER.set(f);
}

pub fn emit(name: &str, data: &Value) {
    if let Some(f) = EMITTER.get() {
        f(name, data);
    }
}
