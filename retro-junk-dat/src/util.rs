/// Simple ISO-8601-ish timestamp without pulling in a chrono dependency.
///
/// Returns an approximate year with Unix timestamp, e.g. "2025-xx-xx (unix: 1234567890)".
pub(crate) fn chrono_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;
    let years = 1970 + days / 365; // approximate
    format!("{years}-xx-xx (unix: {secs})")
}
