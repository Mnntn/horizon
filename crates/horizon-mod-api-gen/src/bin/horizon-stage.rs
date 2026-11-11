//! `horizon-stage ROOT POINTS_JSON INTERNALS_JSON API_VERSION`: mark points and expose internals in a staged
//! source copy, then write both manifests (#589). Never run on a working tree.
use horizon_mod_api_gen::staging::{self, Mode};
use std::path::PathBuf;

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let usage = || {
        eprintln!(
            "usage: horizon-stage ROOT POINTS_JSON INTERNALS_JSON API_VERSION [--sync-internals]"
        );
        std::process::exit(2)
    };
    let [root, list, internals, api_version, options @ ..] = &args[..] else {
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
    match staging::stage(root, list, internals, mode, api_version) {
        Ok(staging::Staged {
            points: staged,
            internals: widened,
        }) => {
            println!(
                "staged {} points ({} with a head only, {} with a raw form) in {} files",
                staged.points, staged.head_only, staged.raw, staged.files
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
