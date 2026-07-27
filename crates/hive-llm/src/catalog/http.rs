//! One shared HTTP client for every catalog request.
//!
//! `reqwest::Client::new()` builds a fresh connection pool and TLS config each
//! time; catalog work fires several requests back to back, so they all share
//! one client and reuse warm connections instead.

use std::sync::OnceLock;
use std::time::Duration;

/// Listing endpoints are small and interactive — fail fast rather than hang the
/// picker on a provider that never answers.
const TIMEOUT: Duration = Duration::from_secs(20);

pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}
