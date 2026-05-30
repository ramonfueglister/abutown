//! Startup-reliability regression guard.
//!
//! `build_app_from_config` loads the base world with `?` (app.rs), so a config
//! pointed at a missing base world must yield a clean `Err`, never a panic.
//!
//! This lives in its own test binary (not `http.rs`) on purpose: the test
//! mutates the process-wide `ABUTOWN_BASE_WORLD_PATH` env var, which
//! `resolve_base_world_path()` reads. Many `http.rs` tests build apps off the
//! default path and run concurrently in the same binary, so mutating the env
//! var there would race and poison them. A dedicated single-test binary keeps
//! the mutation isolated. (The crate has no `serial_test` dependency.)

use sim_server::app::build_app_from_config;
use sim_server::config::ServerConfig;

#[tokio::test]
async fn build_app_from_config_errors_on_missing_base_world() {
    // SAFETY: this is the only test in this binary, so no concurrent reader of
    // ABUTOWN_BASE_WORLD_PATH can observe the mutation.
    unsafe {
        std::env::set_var("ABUTOWN_BASE_WORLD_PATH", "/nonexistent/abutopia-xyz");
    }

    // The base-world load (`?`) is the first fallible step in
    // build_app_from_config, before any DB connection, so a dummy database_url
    // is fine — we never reach it.
    let cfg = ServerConfig {
        database_url: "postgres://unused.invalid/db".to_string(),
        supabase_url: "http://dummy.local".to_string(),
        cors_allowed_origins: Vec::new(),
    };

    let result = build_app_from_config(&cfg).await;

    unsafe {
        std::env::remove_var("ABUTOWN_BASE_WORLD_PATH");
    }

    assert!(
        result.is_err(),
        "missing base world must be a clean Err, not a panic"
    );
}
