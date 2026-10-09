fn main() {
    println!("cargo:rerun-if-env-changed=WEATHER_BRIDGE_BUILD_REVISION");
    let revision = match std::env::var("WEATHER_BRIDGE_BUILD_REVISION") {
        Ok(revision)
            if revision.len() == 40
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            revision
        }
        Ok(_) => panic!(
            "WEATHER_BRIDGE_BUILD_REVISION must be exactly 40 lowercase hexadecimal characters"
        ),
        Err(std::env::VarError::NotPresent) => "unknown".to_owned(),
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("WEATHER_BRIDGE_BUILD_REVISION must be valid UTF-8")
        }
    };
    // Keep Cargo's line protocol safe at the output boundary as well as validation.
    println!(
        "cargo:rustc-env=WEATHER_BRIDGE_BUILD_REVISION_EMBEDDED={}",
        revision.replace(['\n', '\r'], "")
    );
}
