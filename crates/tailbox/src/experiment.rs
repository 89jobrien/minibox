static TSRS_EXPERIMENT_SET: std::sync::Once = std::sync::Once::new();

/// Set `TS_RS_EXPERIMENT=this_is_unstable_software` exactly once at startup.
///
/// `tailscale-rs` requires this variable to be set before any `Device` is
/// constructed. Called at the top of `TailnetNetwork::setup()`.
///
/// # Safety
///
/// `SAFETY:` `set_var` is called inside a `Once` block. The only callers of
/// `TailnetNetwork::setup()` are async tasks dispatched by `miniboxd` after
/// tracing + adapter initialisation is complete. No other thread reads or
/// writes `TS_RS_EXPERIMENT`. The `Once` guarantee prevents concurrent
/// calls, satisfying the Rust 2024 requirement that `set_var` be called in
/// a context where no other threads are reading the env.
pub fn ensure_tsrs_experiment() {
    TSRS_EXPERIMENT_SET.call_once(|| {
        // SAFETY: called exactly once; no concurrent readers of TS_RS_EXPERIMENT.
        unsafe {
            std::env::set_var("TS_RS_EXPERIMENT", "this_is_unstable_software");
        }
    });
}
