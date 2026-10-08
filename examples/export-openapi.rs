//! Export the generated contract without starting a server or fetching weather.
use std::{env, fs, path::PathBuf};
fn main() -> anyhow::Result<()> {
    let mut args = env::args_os().skip(1);
    let first = args.next();
    let check = first.as_deref() == Some(std::ffi::OsStr::new("--check"));
    let path = if check {
        args.next().unwrap_or_else(|| "openapi.json".into())
    } else {
        first.unwrap_or_default()
    };
    anyhow::ensure!(
        args.next().is_none(),
        "usage: export-openapi [--check] [path]"
    );
    let document = weather_bridge::api::openapi_json();
    if path.is_empty() {
        print!("{document}");
    } else {
        let path = PathBuf::from(path);
        if check {
            anyhow::ensure!(
                fs::read_to_string(&path)? == document,
                "OpenAPI artifact is stale; run cargo run --locked --example export-openapi -- {}",
                path.display()
            );
        } else {
            // Generate fully before replacing the artifact; a build/generation failure
            // cannot truncate the checked-in file.
            let mut temporary = path.as_os_str().to_os_string();
            temporary.push(format!(".{}.tmp", std::process::id()));
            fs::write(&temporary, document)?;
            fs::rename(&temporary, &path)?;
        }
    }
    Ok(())
}
