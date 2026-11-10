//! Private source call sites share the listed-site compiler and runtime registry.
use super::{Declaration, Site};
use std::{fs, path::Path};

/// Generated source-only metadata, never part of the published points catalog.
pub const FILE: &str = "source-sites.json";

/// Read private call records for generated IDs and native facade construction.
pub fn read(root: &Path) -> Result<Vec<Site>, String> {
    let path = root.join(FILE);
    match fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// The sole facade name for this canonical call place, independent of written aliases.
pub fn facade_name(target: &str, callee: &str, ordinal: usize) -> String {
    super::rewrite::name(
        &Declaration {
            expression: None,
            local: None,
            target: crate::points::plain_path(target),
            selector: crate::points::plain_path(callee),
            ordinal: Some(ordinal),
            ctx: String::new(),
            origin: String::new(),
        },
        ordinal,
    )
}

/// Resolve every request with U2 before writing source or starting the game build.
pub fn stage(
    root: &Path,
    entries: &[crate::internals::Internal],
    requests: &[Declaration],
) -> Result<Vec<Site>, String> {
    let sites = super::stage::stage_declarations(root, entries, requests, true)
        .map_err(|error| error.to_string())?;
    fs::write(
        root.join(FILE),
        serde_json::to_vec(&sites).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(sites)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crossing::tests::internal;

    #[test]
    fn private_sites_keep_ids_and_validate_identity_before_writing() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/source-calls-{}", std::process::id()));
        let file = root.join("crates/horizon-client/src/game.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join("crates/horizon-mod-api-gen/slots")).unwrap();
        let text = "struct Game; impl Game { #[inline(always)] fn run(&mut self) -> u32 { self.step(2); 0 } fn step(&mut self, x: u32) -> u32 { x + 1 } }";
        let entries = vec![
            internal("horizon_client::game::Game", "struct", "struct Game;"),
            internal(
                "horizon_client::game::Game::run",
                "method",
                "impl Game { fn run(&mut self) -> u32 }",
            ),
            internal(
                "horizon_client::game::Game::step",
                "method",
                "impl Game { fn step(&mut self, x: u32) -> u32 }",
            ),
        ];
        let mut request = Declaration {
            expression: None,
            local: None,
            target: entries[1].path.clone(),
            selector: entries[2].path.clone(),
            ordinal: Some(0),
            ctx: String::new(),
            origin: "one:call".into(),
        };
        let (_, discovered) = super::super::rewrite::rewrite(
            text,
            "horizon_client::game",
            &entries,
            std::slice::from_ref(&request),
            true,
        )
        .unwrap();
        request.ctx = discovered[0].ctx.clone();
        fs::write(&file, text).unwrap();
        let sites = stage(&root, &entries, std::slice::from_ref(&request)).unwrap();
        assert_eq!(sites.len(), 1);
        assert_eq!(read(&root).unwrap()[0].ctx, request.ctx);
        assert!(!root.join("points.json").exists());
        assert!(
            super::super::stage::ids(&root)
                .unwrap()
                .contains(&sites[0].name)
        );
        assert!(
            super::super::slots(&root.join(super::super::DIRECTORY))
                .unwrap()
                .calls
                .is_empty()
        );
        syn::parse_file(&fs::read_to_string(&file).unwrap()).unwrap();
        for (changed, expected) in [
            (
                text.replace("self.step(2)", "self.step(1); self.step(2)"),
                "E_INJECT_MOVED",
            ),
            (
                text.replace("self.step(2)", "self.step(3)"),
                "E_INJECT_LOST",
            ),
            (text.replace("self.step(2)", "0"), "E_INJECT_LOST"),
            (
                text.replace("self.step(2)", "self.step(2); self.step(2)"),
                "E_INJECT_LOST",
            ),
        ] {
            fs::write(&file, &changed).unwrap();
            assert!(
                stage(&root, &entries, std::slice::from_ref(&request))
                    .unwrap_err()
                    .contains(expected)
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), changed);
        }
        fs::write(&file, text).unwrap();
        let mut alias = request;
        alias.selector = "self.step".into();
        assert!(
            stage(&root, &entries, &[alias])
                .unwrap_err()
                .contains("canonical callee")
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), text);
        fs::remove_dir_all(root).unwrap();
    }
}
