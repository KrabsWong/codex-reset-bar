//! The Codex Resets API, as this app reads it.
//!
//! Two endpoints: the current state (`/api/v1/status`) and the reset
//! announcements (`/api/v1/resets`, cursor-paginated).
//!
//! Data from <a href="https://codex-resets.com">Codex Resets</a>.

use nsh_core::ui::prelude::*;

use crate::model::{ResetPage, StatusEnvelope};

/// Where the data comes from (also the attribution link).
pub const SITE: &str = "https://codex-resets.com";

/// `SITE`, or `CODEX_RESET_API_BASE` for a local server or a test.
fn site() -> String {
    std::env::var("CODEX_RESET_API_BASE").unwrap_or_else(|_| SITE.to_owned())
}

/// The current reset status.
pub async fn status() -> Result<crate::model::StatusData, String> {
    let envelope: StatusEnvelope = get("/api/v1/status").await?;
    Ok(envelope.data)
}

/// A page of reset announcements, newest first.
pub async fn resets(limit: u32, cursor: Option<&str>) -> Result<ResetPage, String> {
    let mut path = format!("/api/v1/resets?limit={limit}&order=desc");
    if let Some(cursor) = cursor {
        path.push_str("&cursor=");
        path.push_str(cursor);
    }
    get(&path).await
}

async fn get<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let url = format!("{}{path}", site());
    // The API permits four hours of client-side caching. Revalidate each
    // poll; our own disk cache already provides the fast startup path.
    let request = nsh_core::platform::HttpRequest::get(url).header("Cache-Control", "no-cache");
    let response = fetch(request)
        .await
        .map_err(|error| format!("Network request failed: {error}"))?;
    if !response.ok() {
        return Err(format!("Server returned HTTP {}", response.status()));
    }
    serde_json::from_str(&response.text()).map_err(|error| format!("Invalid response: {error}"))
}
