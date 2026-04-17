static TSRS_EXPERIMENT_SET: std::sync::Once = std::sync::Once::new();

/// Set `TS_RS_EXPERIMENT=this_is_unstable_software` exactly once at startup.
///
/// `tailscale-rs` requires this variable to be set before any `Device` is
/// constructed. Called at the top of `TailnetNetwork::setup()`.
///
/// # Safety
///
/// `set_var` is called inside a `Once` block, guaranteeing it runs exactly
/// once. Callers of `TailnetNetwork::setup()` must not call `set_var` or
/// `remove_var` on `TS_RS_EXPERIMENT` concurrently. Reads by tailscale-rs
/// are safe concurrently with a completed `set_var` — the `Once::call_once`
/// ensures the write is fully visible before any `Device` construction
/// proceeds.
pub fn ensure_tsrs_experiment() {
    TSRS_EXPERIMENT_SET.call_once(|| {
        // SAFETY: called exactly once via `Once`; no concurrent set_var/remove_var
        // on TS_RS_EXPERIMENT. Reads by tailscale-rs are safe concurrently with a
        // completed set_var — the Once::call_once ensures the write is fully visible
        // before any Device construction proceeds.
        unsafe {
            std::env::set_var("TS_RS_EXPERIMENT", "this_is_unstable_software");
        }
    });
}
