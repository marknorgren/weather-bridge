//! Command-line front end. Behavior lives in the shared `Weather` service; this module only
//! parses arguments, prints results and maps errors to exit codes.
mod args;
mod render;

pub use args::{Cli, Command};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{self, Write},
    process::ExitCode,
    sync::Arc,
};
use weather_bridge::{Error, cities::Cities, envelope, model::Completeness, weather::Weather};

/// A successful result in both output formats.
struct Rendered {
    envelope: Value,
    text: String,
    /// False when alerts could not be checked or a source is missing.
    complete: bool,
}
impl Rendered {
    fn new<T: Serialize>(data: T, text: impl FnOnce(&T) -> String) -> Self {
        Self {
            text: text(&data),
            envelope: envelope(data),
            complete: true,
        }
    }
    /// A weather result, which may be incomplete.
    fn weather<T: Serialize + Completeness>(data: T, text: impl FnOnce(&T) -> String) -> Self {
        let complete = data.is_complete();
        Self {
            complete,
            ..Self::new(data, text)
        }
    }
}

enum Failure {
    Service(Error),
    Other(anyhow::Error),
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Service(error)
    }
}
impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self::Other(error)
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON values serialize")
}

/// Run `cities`, `weather`, `hourly` or `alerts`. The service is built only for commands
/// that call NWS; `cities` reads the embedded index and never creates an HTTP client.
/// Service errors print readably to stderr, or as the JSON error body to stdout with
/// `--json`, and map to an exit code. An incomplete weather result still prints but
/// exits 3, so scripts can tell a failed alert check from no alerts.
pub async fn run(
    command: Command,
    weather: impl FnOnce() -> anyhow::Result<Arc<Weather>>,
) -> ExitCode {
    let json = matches!(
        &command,
        Command::Cities { json: true, .. }
            | Command::Weather(args::Lookup { json: true, .. })
            | Command::Hourly(args::Lookup { json: true, .. })
            | Command::Alerts(args::Lookup { json: true, .. })
    );
    let result = execute(command, weather).await;
    print_result(
        result,
        json,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

/// A consumer closing stdout is ordinary pipeline termination. Preserve the command's
/// success, partial-result or service-error status; other output failures exit 1.
fn print_result(
    result: Result<Rendered, Failure>,
    json: bool,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> ExitCode {
    let (output, status, to_stdout) = match result {
        Ok(out) => {
            let text = if json {
                format!("{}\n", pretty(&out.envelope))
            } else {
                out.text
            };
            (text, render::success_exit_code(out.complete), true)
        }
        Err(Failure::Service(error)) => {
            let text = if json {
                format!("{}\n", pretty(&error.body()))
            } else {
                render::error_text(&error)
            };
            (text, render::exit_code(error.code), json)
        }
        Err(Failure::Other(error)) => (format!("error: {error:#}\n"), 1, false),
    };
    let write = if to_stdout {
        stdout
            .write_all(output.as_bytes())
            .and_then(|()| stdout.flush())
    } else {
        stderr
            .write_all(output.as_bytes())
            .and_then(|()| stderr.flush())
    };
    match write {
        Ok(()) => ExitCode::from(status),
        Err(error) if to_stdout && error.kind() == io::ErrorKind::BrokenPipe => {
            ExitCode::from(status)
        }
        Err(error) => {
            let _ = writeln!(stderr, "error: could not write output: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(
    command: Command,
    weather: impl FnOnce() -> anyhow::Result<Arc<Weather>>,
) -> Result<Rendered, Failure> {
    Ok(match command {
        Command::Cities { query, .. } => Rendered::new(Cities::load()?.search(&query)?, |found| {
            render::cities_table(found)
        }),
        Command::Weather(lookup) => Rendered::weather(
            weather()?.report(lookup.query()).await?,
            render::weather_text,
        ),
        Command::Hourly(lookup) => Rendered::weather(
            weather()?.hourly(lookup.query()).await?,
            render::hourly_text,
        ),
        Command::Alerts(lookup) => Rendered::weather(
            weather()?.alerts(lookup.query()).await?,
            render::alerts_text,
        ),
        Command::Serve { .. } | Command::Mcp => unreachable!("long-running commands run in main"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailedWriter(io::ErrorKind);
    impl Write for FailedWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn result(complete: bool) -> Result<Rendered, Failure> {
        Ok(Rendered {
            envelope: serde_json::json!({}),
            text: "report\n".into(),
            complete,
        })
    }

    #[test]
    fn broken_pipe_preserves_partial_result_status() {
        let mut stderr = Vec::new();
        let status = print_result(
            result(false),
            false,
            &mut FailedWriter(io::ErrorKind::BrokenPipe),
            &mut stderr,
        );
        assert_eq!(status, ExitCode::from(3));
        assert!(stderr.is_empty());
    }

    #[test]
    fn other_output_errors_exit_1_instead_of_panicking() {
        let mut stderr = Vec::new();
        let status = print_result(
            result(true),
            false,
            &mut FailedWriter(io::ErrorKind::PermissionDenied),
            &mut stderr,
        );
        assert_eq!(status, ExitCode::FAILURE);
        assert!(
            String::from_utf8(stderr)
                .unwrap()
                .contains("could not write output")
        );
        let status = print_result(
            Err(Failure::Service(Error::invalid("bad query"))),
            false,
            &mut Vec::new(),
            &mut FailedWriter(io::ErrorKind::BrokenPipe),
        );
        assert_eq!(status, ExitCode::FAILURE);
    }
}
