//! `horizon-stage ROOT INTERNALS_JSON API_VERSION`: expose internals and give functions their
//! replacement slots in a staged source copy, then write the internals manifest and, beside it,
//! the SDK catalog. Never run on a working tree.
use horizon_mod_api_gen::staging::{self, Mode};
use std::path::PathBuf;

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let usage = || {
        eprintln!("usage: horizon-stage ROOT INTERNALS_JSON API_VERSION [--sync-internals]");
        std::process::exit(2)
    };
    let [root, internals, api_version, options @ ..] = &args[..] else {
        usage()
    };
    // Discovery never builds or publishes a game.
    let mode = match options {
        [] => Mode::Build,
        [flag] if flag == "--sync-internals" => Mode::Discovery,
        _ => usage(),
    };
    let Some(api_version) = api_version.to_str() else {
        usage()
    };
    match staging::stage(root, internals, mode, api_version) {
        Ok(staging::Staged {
            replace: staged,
            internals: widened,
        }) => {
            println!(
                "gave {} functions a replacement slot in {} files of {} crates",
                staged.functions, staged.files, staged.crates
            );
            let shapes = widened
                .entries
                .iter()
                .filter(|entry| entry.shape_only)
                .count();
            println!(
                "exposed {} internals and listed {} type shapes in {} changed files: {:?}",
                widened.entries.len() - shapes,
                shapes,
                widened.files,
                widened.changed
            );
        }
        Err(error) => {
            eprintln!("horizon-stage: {error}");
            std::process::exit(1);
        }
    }
}
