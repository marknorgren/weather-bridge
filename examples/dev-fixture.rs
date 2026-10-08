//! Offline, synthetic development server using the same HTTP and MCP app as production.
#[path = "../tests/common/mod.rs"]
mod common;

use common::{DevScenario, dev_app, dev_fixture};
use std::{process::ExitCode, str::FromStr, sync::Arc, time::Duration};
use tokio::sync::Notify;

const DEFAULT_BIND: &str = "127.0.0.1:8790";
const DRAIN: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("SYNTHETIC OFFLINE FIXTURE error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let scenario = parse_scenario()?;
    let fixture = dev_fixture(scenario).await;
    let bind = std::env::var("WEATHER_BRIDGE_DEV_BIND").unwrap_or_else(|_| DEFAULT_BIND.into());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    eprintln!(
        "SYNTHETIC OFFLINE FIXTURE [{}] ready at http://{bind}",
        scenario.as_str()
    );
    eprintln!("SYNTHETIC OFFLINE FIXTURE uses no live NWS data; press Ctrl-C to stop");

    let stopping = Arc::new(Notify::new());
    let signalled = stopping.clone();
    let server = axum::serve(listener, dev_app(fixture.weather.clone(), scenario))
        .with_graceful_shutdown(async move {
            shutdown().await;
            signalled.notify_one();
        });
    tokio::select! {
        result = server => result?,
        () = async { stopping.notified().await; tokio::time::sleep(DRAIN).await } => {
            eprintln!("SYNTHETIC OFFLINE FIXTURE shutdown drain timed out; closing connections");
        }
    }
    Ok(())
}

fn parse_scenario() -> anyhow::Result<DevScenario> {
    let mut args = std::env::args().skip(1);
    let value = args.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: cargo run --locked --example dev-fixture -- <healthy|stale|alerts-unavailable>"
        )
    })?;
    anyhow::ensure!(
        args.next().is_none(),
        "usage: cargo run --locked --example dev-fixture -- <healthy|stale|alerts-unavailable>"
    );
    DevScenario::from_str(&value).map_err(anyhow::Error::msg)
}

async fn shutdown() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {_ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {}}
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}
