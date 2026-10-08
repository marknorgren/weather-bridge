//! Streamable HTTP transport for the MCP server at `/mcp`.
use super::McpAccess;
use crate::{mcp::WeatherMcp, weather::Weather};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::sync::Arc;

fn mcp_config(access: McpAccess) -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_allowed_origins(["http://localhost:*", "http://127.0.0.1:*", "http://[::1]:*"]);
    if let Some(hosts) = access.hosts {
        config = config.with_allowed_hosts(hosts);
    }
    if let Some(origins) = access.origins {
        config = config.with_allowed_origins(origins);
    }
    config
}
/// The stateless MCP service, one `WeatherMcp` per request over the shared service.
pub(super) fn service(
    weather: Arc<Weather>,
    access: McpAccess,
) -> StreamableHttpService<WeatherMcp, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(WeatherMcp::new(weather.clone())),
        LocalSessionManager::default().into(),
        mcp_config(access),
    )
}
