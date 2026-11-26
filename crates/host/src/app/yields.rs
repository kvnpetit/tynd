use std::collections::HashMap;

use smallvec::SmallVec;
use wry::WebView;

use super::{child_webview::ChildWebviewEntry, SecondaryEntry, PRIMARY_LABEL};
use crate::webview::shim;

/// Maximum yields buffered per window before we flush synchronously.
/// Higher = fewer `evaluate_script` calls, lower = smaller memory spike.
pub(super) const YIELD_BATCH_MAX: usize = 64;
/// How long the first buffered yield waits for siblings before flushing.
/// Invisible to users (<1 frame) but cuts per-chunk overhead on bursty streams.
pub(super) const YIELD_FLUSH_MS: u64 = 10;

/// Per-window yield bucket. Inline capacity of 8 covers a typical streaming
/// burst between two 10 ms timer ticks; bucket caps at `YIELD_BATCH_MAX` (64)
/// via the synchronous-flush branch.
pub(super) type YieldBucket = SmallVec<[(String, String); 8]>;

/// Drain the yield buffer into one `evaluate_script` per webview. Single-entry
/// buckets take the `eval_yield` fast path (plain `format!`) to skip the 2-D
/// batch envelope.
pub(super) fn flush_yields(
    buffer: &mut HashMap<String, YieldBucket>,
    primary: &WebView,
    secondaries: &HashMap<String, SecondaryEntry>,
    child_webviews: &HashMap<String, ChildWebviewEntry>,
) {
    for (label, pairs) in buffer.drain() {
        if pairs.is_empty() {
            continue;
        }
        let Some(wv) = webview_for(&label, primary, secondaries, child_webviews) else {
            crate::tynd_warn!("dropping yields for unknown webview label={label:?}");
            continue;
        };
        if pairs.len() == 1 {
            let (id, value) = &pairs[0];
            if let Err(e) = wv.evaluate_script(&shim::eval_yield(id, value)) {
                crate::tynd_warn!("yield eval failed for id={id} label={label:?}: {e}");
            }
        } else if let Err(e) = wv.evaluate_script(&shim::eval_yield_batch(&pairs)) {
            crate::tynd_warn!(
                "yield batch eval failed for label={label:?} count={n}: {e}",
                n = pairs.len()
            );
        }
    }
}

pub(super) fn webview_for<'a>(
    label: &str,
    primary: &'a WebView,
    secondaries: &'a HashMap<String, SecondaryEntry>,
    child_webviews: &'a HashMap<String, ChildWebviewEntry>,
) -> Option<&'a WebView> {
    if label == PRIMARY_LABEL {
        Some(primary)
    } else {
        secondaries
            .get(label)
            .map(|entry| &entry.webview)
            .or_else(|| child_webviews.get(label).map(|entry| &entry.webview))
    }
}
