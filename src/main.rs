mod cli;

use clap::Parser;
use cli::{Cli, Command};
use rmcp::ServiceExt;
use std::{process::ExitCode, sync::Arc, time::Duration};
use tokio::sync::Notify;
use weather_bridge::{
    api::{HttpConfig, McpAccess},
    mcp::WeatherMcp,
    weather::{Config, Weather},
};

/// How long in-flight requests get to finish after a shutdown signal.
const DRAIN: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    let user_agent = cli.user_agent;
    let weather = move || {
        Ok::<_, anyhow::Error>(Arc::new(Weather::configured(Config {
            user_agent,
            ..Config::default()
        })?))
    };
    match cli.command {
        Command::Mcp => finish(serve_mcp(weather).await),
        Command::Serve {
            bind,
            mcp_hosts,
            mcp_origins,
            origin_verify,
            docs_url,
        } => finish(
            async {
                let weather = weather()?;
                let listener = tokio::net::TcpListener::bind(bind).await?;
                tracing::info!(
                    %bind,
                    version = env!("CARGO_PKG_VERSION"),
                    revision = weather_bridge::BUILD_REVISION,
                    "Weather Bridge ready"
                );
                let stopping = Arc::new(Notify::new());
                let signalled = stopping.clone();
                let server = axum::serve(
                    listener,
                    weather_bridge::api::app(
                        weather,
                        HttpConfig {
                            mcp_access: McpAccess {
                                hosts: mcp_hosts,
                                origins: mcp_origins,
                            },
                            origin_verify,
                            docs_url,
                        },
                    ),
                )
                .with_graceful_shutdown(async move {
                    shutdown().await;
                    signalled.notify_one();
                });
                // Graceful shutdown waits for open connections; bound that wait.
                tokio::select! {
                    result = server => result?,
                    () = async { stopping.notified().await; tokio::time::sleep(DRAIN).await } => {
                        tracing::warn!("Shutdown drain timed out; closing remaining connections");
                    }
                }
                Ok(())
            }
            .await,
        ),
        command => cli::run(command, weather).await,
    }
}

/// Human-readable local logs and one-object-per-line JSON in the AWS deployment. Both
/// formats write only to stderr, which keeps MCP stdio stdout reserved for protocol data.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "weather_bridge=info,tower_http=warn".into());
    if std::env::var_os("AWS_LAMBDA_FUNCTION_NAME").is_some() {
        tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .compact()
            .with_writer(std::io::stderr)
            .with_env_filter(filter)
            .init();
    }
}

async fn serve_mcp(weather: impl FnOnce() -> anyhow::Result<Arc<Weather>>) -> anyhow::Result<()> {
    let running = WeatherMcp::new(weather()?)
        .serve(rmcp::transport::stdio())
        .await?;
    tokio::select! {result=running.waiting()=>{result?;},()=shutdown()=>{}}
    Ok(())
}

/// Startup and runtime failures of the long-running commands exit 1.
fn finish(result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=sig.recv()=>{}}
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}
