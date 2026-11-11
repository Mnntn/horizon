use super::rewrite::rewrite;
use super::*;
use crate::{internals::Internal, points};
use quote::{format_ident, quote};

/// The list directory and each listed crate's source: build-script inputs.
pub fn tracked(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut paths = vec![root.join(DIRECTORY)];
    paths.extend(
        files(root, &slots(&root.join(DIRECTORY))?.calls)?
            .into_iter()
            .map(|file| file.path),
    );

    Ok(paths)
}

fn files(root: &Path, declarations: &[Declaration]) -> Result<Vec<points::GameFile>, Error> {
    let mut files =
        points::game_files(&root.join("crates/horizon-client/src"), true).map_err(Cause::Source)?;
    let crates: std::collections::BTreeSet<_> = declarations
        .iter()
        .filter_map(|d| d.target.split("::").next().map(str::to_owned))
        .collect();
    for krate in crates.into_iter().filter(|k| k != "horizon_client") {
        if points::below_points(root)
            .map_err(Cause::Source)?
            .contains(&krate)
        {
            let declaration = declarations
                .iter()
                .find(|d| d.target.starts_with(&format!("{krate}::")))
                .expect("listed crate");
            return Err(Error::from(Cause::Signature(
                "site crate is built under the points' own crate",
            ))
            .at(declaration.origin.clone()));
        }
        let (_, lib) = points::crate_root(root, &krate).map_err(Cause::Source)?;
        let dir = lib.parent().expect("source parent").to_owned();
        files.extend(points::module_files(krate, lib, dir, true).map_err(Cause::Source)?);
    }
    Ok(files)
}

/// Generate site IDs independently of the raw types, keeping the points crate below gameplay.
pub fn ids(root: &Path) -> Result<String, Error> {
    let declarations = slots(&root.join(DIRECTORY))?.calls;
    let names = declarations
        .iter()
        .map(|d| format_ident!("{}", super::rewrite::name(d, d.ordinal.unwrap_or(0))))
        .collect::<Vec<_>>();
    Ok(quote! {
        #[doc(hidden)] #[repr(usize)] #[derive(Clone, Copy, Debug, PartialEq, Eq)] #[non_exhaustive]
        pub enum SiteId { #(#names,)* __Count }
        #[doc(hidden)] pub const SITE_COUNT: usize = SiteId::__Count as usize;
    }
    .to_string())
}

/// Validate the entire union before writing any expression rewrite. Widening precedes this pass.
pub fn stage(root: &Path, entries: &[Internal]) -> Result<Vec<Site>, Error> {
    let declarations = &slots(&root.join(DIRECTORY))?.calls;
    if declarations.is_empty() {
        return Ok(Vec::new());
    }
    let rule = crate::crossing::Rule::new(entries);
    let targets = declarations
        .iter()
        .map(|d| {
            super::identity::enclosing(&d.target, entries, &rule)
                .map(|entry| (d, entry))
                .map_err(|error| error.at(format!("{}: {}", d.origin, d.target)))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut changes = Vec::new();
    let mut sites = Vec::new();
    let mut packages = std::collections::BTreeSet::new();
    for file in files(root, declarations)? {
        let selected = targets
            .iter()
            .filter(|(_, entry)| entry.path.starts_with(&format!("{}::", file.module)))
            .map(|(declaration, _)| (*declaration).clone())
            .collect::<Vec<_>>();
        if selected.is_empty() {
            continue;
        }
        let source = fs::read_to_string(&file.path)?;
        let source = crate::game_modules::expand(&file.path, &source).map_err(Cause::Source)?;
        let (rewritten, found) = rewrite(&source, &file.module, entries, &selected)?;
        if found.is_empty() {
            continue;
        }
        if !file.module.starts_with("horizon_client::") {
            packages.insert(
                points::crate_root(root, file.module.split("::").next().expect("crate"))
                    .map_err(Cause::Source)?
                    .0,
            );
        }
        sites.extend(found);
        changes.push((file.path, rewritten));
    }
    for d in declarations {
        if !sites.iter().any(|s| {
            s.target == d.target
                && s.selector == d.selector
                && Some(s.ordinal) == d.ordinal.or(Some(0))
        }) {
            return Err(Error::from(Cause::Missing(
                "E_INJECT_MISSING: no eligible function matched the site target",
            ))
            .at(format!("{}: {}", d.origin, d.target)));
        }
    }
    let mut temps = Vec::new();
    for (path, text) in &changes {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("site");
        let temp = path.with_file_name(format!(".{name}.site-tmp"));
        if let Err(error) = fs::write(&temp, text) {
            for (temp, _) in &temps {
                let _ = fs::remove_file(temp);
            }
            return Err(error.into());
        }
        temps.push((temp, path));
    }
    if let Err(error) = points::dependencies(root, packages) {
        for (temp, _) in &temps {
            let _ = fs::remove_file(temp);
        }
        return Err(Cause::Source(error).into());
    }
    for index in 0..temps.len() {
        if let Err(error) = fs::rename(&temps[index].0, temps[index].1) {
            for (temp, _) in &temps[index..] {
                let _ = fs::remove_file(temp);
            }
            return Err(error.into());
        }
    }
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sites)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crossing::tests::internal;

    #[test]
    fn site_stage_follows_nominal_owner_and_keeps_physical_writes() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("site-area-{}", std::process::id()));
        let game = root.join("crates/horizon-client/src/game.rs");
        let area = root.join("crates/horizon-client/src/game/runtime");
        fs::create_dir_all(&area).unwrap();
        fs::write(area.join(".game-area"), "").unwrap();
        let directory = root.join(DIRECTORY);
        fs::create_dir_all(&directory).unwrap();
        let declaration = directory.join("test.txt");
        fs::write(
            &declaration,
            "horizon_client::game::Machine::run | self.helper | 0 |\n",
        )
        .unwrap();
        let base = "struct Machine;\n";
        let method = "impl Machine { fn helper(&self, value: u32) -> u32 { value + 1 } fn run(&self) -> u32 { self.helper(2) } }";
        let signature = "impl Machine { fn run(&self) -> u32 }";
        let mut entries = vec![
            internal("horizon_client::game::Machine", "struct", "struct Machine;"),
            internal(
                "horizon_client::game::Machine::helper",
                "method",
                "impl Machine { fn helper(&self, value: u32) -> u32 }",
            ),
            internal("horizon_client::game::Machine::run", "method", signature),
        ];
        fs::write(&game, format!("{base}{method}")).unwrap();
        let before = stage(&root, &entries).unwrap().remove(0);
        let root_source = format!("{base}{}\n", crate::game_modules::INCLUDE);
        fs::write(&game, &root_source).unwrap();
        let moved = area.join("feature.rs");
        let source = format!("use super::*; {method}");
        fs::write(&moved, &source).unwrap();
        entries[1].path = "horizon_client::game::feature::Machine::helper".into();
        entries[2].path = "horizon_client::game::feature::Machine::run".into();
        for entry in &mut entries[1..] {
            entry
                .refs
                .insert("Machine".into(), "horizon_client::game::Machine".into());
        }
        let after = stage(&root, &entries).unwrap().remove(0);
        assert_eq!(
            (after.name.as_str(), after.ctx.as_str(), after.hash.as_str()),
            (
                before.name.as_str(),
                before.ctx.as_str(),
                before.hash.as_str()
            )
        );
        assert_eq!(after.target, "horizon_client::game::Machine::run");
        assert_eq!(after.call, before.call);
        assert_eq!(after.call, "horizon_client::game::Machine::helper");
        assert_eq!(fs::read_to_string(&game).unwrap(), root_source);
        let rewritten = fs::read_to_string(&moved).unwrap();
        syn::parse_file(&rewritten).unwrap();
        assert!(rewritten.contains(&after.name), "{rewritten}");
        // A genuine missing call remains Lost, even though its nominal owner still matches.
        fs::write(&moved, source.replace("self.helper(2)", "2")).unwrap();
        fs::write(
            &declaration,
            format!(
                "horizon_client::game::Machine::run | self.helper | 0 | {}\n",
                before.ctx
            ),
        )
        .unwrap();
        let error = stage(&root, &entries).unwrap_err().to_string();
        assert!(error.contains("E_INJECT_LOST"), "{error}");
        assert!(!fs::read_to_string(&moved).unwrap().contains(&after.name));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn site_stage_validates_all_targets_before_writing() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("site-stage-{}", std::process::id()));
        let file = root.join("crates/horizon-client/src/game.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let directory = root.join(DIRECTORY);
        fs::create_dir_all(&directory).unwrap();
        let source = "fn run() -> u32 { add(2) } fn add(value: u32) -> u32 { value + 1 } struct Game; trait Ext { fn run(); fn helper() -> u32; } impl Ext for Game { fn run() { Self::helper(); } fn helper() -> u32 { 7 } }";
        fs::write(&file, source).unwrap();
        let entries = vec![
            internal("horizon_client::game::run", "fn", "fn run() -> u32"),
            internal(
                "horizon_client::game::add",
                "fn",
                "fn add(value: u32) -> u32",
            ),
        ];
        let list = directory.join("test.txt");
        fs::write(
            &list,
            "horizon_client::game::run | add | 0 |\nhorizon_client::game::Game::run | Self::helper | 0 |\n",
        )
        .unwrap();
        let error = stage(&root, &entries).unwrap_err();
        assert!(
            error.to_string().contains("test.txt:2")
                && error.to_string().contains("no eligible function"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), source);
        fs::write(&list, "horizon_client::game::run | add | 0 |\n").unwrap();
        let sites = stage(&root, &entries).unwrap();
        assert_eq!(sites.len(), 1);
        assert!(ids(&root).unwrap().contains(&sites[0].name));
        let rewritten = fs::read_to_string(&file).unwrap();
        syn::parse_file(&rewritten).unwrap();
        fs::write(&file, source).unwrap();
        fs::write(
            &list,
            format!("horizon_client::game::run | add | 0 | {}\n", sites[0].ctx),
        )
        .unwrap();
        let pinned = stage(&root, &entries).unwrap();
        assert_eq!(pinned[0].hash, sites[0].hash);
        assert_eq!(pinned[0].name, sites[0].name);
        assert_eq!(fs::read_to_string(&file).unwrap(), rewritten);
        fs::remove_dir_all(root).unwrap();
    }
}
