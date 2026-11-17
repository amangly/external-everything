// Minimal local diagnostic sink for the Palepale renderer copied at
// 6490070271354e2a13a30e107065338c530a7322 (build/reference/palepale).
// Call sites are state transitions/errors, rather than per-frame telemetry.
pub fn line(msg: &str) {
    eprintln!("[external-renderer] {msg}");
}
