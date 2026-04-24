/// Initialise the global tracing subscriber.
///
/// Reads `MINIBOX_TRACE_LEVEL` to build a [`tracing_subscriber::EnvFilter`].
/// Falls back to `"info"` when the variable is absent or empty.
///
/// Uses [`tracing_subscriber::fmt().try_init()`] so that calling this function
/// more than once (e.g. in tests) is safe — subsequent calls are silently
/// ignored rather than panicking.
///
/// # Example
///
/// ```ignore
/// minibox_core::init_tracing();
/// ```
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let level = std::env::var("MINIBOX_TRACE_LEVEL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "info".to_string());

    let filter = EnvFilter::new(&level);

    // Ignore the error — a subscriber is already installed (common in tests).
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serialize env mutations across parallel test threads.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn init_tracing_does_not_panic() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: serialised by ENV_LOCK; no concurrent env mutation.
        unsafe { std::env::remove_var("MINIBOX_TRACE_LEVEL") };
        // Must not panic even when called multiple times.
        init_tracing();
        init_tracing();
    }

    #[test]
    fn init_tracing_respects_env_var() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: serialised by ENV_LOCK; no concurrent env mutation.
        unsafe { std::env::set_var("MINIBOX_TRACE_LEVEL", "debug") };
        init_tracing();
        unsafe { std::env::remove_var("MINIBOX_TRACE_LEVEL") };
    }

    #[test]
    fn init_tracing_falls_back_when_var_is_empty() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: serialised by ENV_LOCK; no concurrent env mutation.
        unsafe { std::env::set_var("MINIBOX_TRACE_LEVEL", "") };
        init_tracing();
        unsafe { std::env::remove_var("MINIBOX_TRACE_LEVEL") };
    }
}
