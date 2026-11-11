//! Executable link policy shared by source executables and prebuilt SDK metadata.

pub fn arguments(target_os: &str, target_env: &str) -> &'static [&'static str] {
    // Windows otherwise gives the main thread 1 MB. Joining splitscreen players constructs
    // a Game deep in the event loop; use 8 MB, as on macOS and Linux.
    match (target_os, target_env) {
        ("windows", "msvc") => &["/STACK:8388608"],
        ("windows", _) => &["-Wl,--stack,8388608"],
        _ => &[],
    }
}
