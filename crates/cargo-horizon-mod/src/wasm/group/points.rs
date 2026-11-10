//! Member-owned point registries: no mod_api registry value crosses a module boundary.
use super::*;
use horizon_mod_package::manifest::inject::{Access, Kind, mod_points};

pub(super) fn prepare(
    members: &mut [Member],
    wire: &std::collections::BTreeSet<String>,
    catalog: Option<&Path>,
) -> Result<Vec<String>, String> {
    let libraries = members
        .iter()
        .map(|m| horizon_mod_api_gen::points::mod_crate::mod_points(&m.root, wire))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::new();
    for index in 0..members.len() {
        let member = &members[index];
        let mut manifest = member.manifest.clone();
        if !manifest.contains_key("wasm") {
            // Inject validation requires a code target; the real wasm pins are written after staging.
            manifest["wasm"]["sha512"] = toml_edit::value("0".repeat(128));
            manifest["wasm"]["toolchain"] = toml_edit::value("pending");
        }
        let parsed = horizon_mod_package::Manifest::parse(&manifest.to_string())?;
        let mut registration = String::new();
        let is_mod = |at: &str| parsed.mod_point_dependency(at).is_some();
        // Native refuses these in `mod_points::route`; here the manifest never names them as mod
        // points, so the same reason is given before they could fall to the game's catalog.
        for injection in parsed.inject.iter().filter(|i| !is_mod(&i.at)) {
            if let Some(owner) = libraries.iter().position(|p| p.owns(&injection.at)) {
                return Err(mod_points::Refusal::LibraryCrate {
                    id: format!("{}:{}", parsed.id, injection.id),
                    library: members[owner].manifest["id"]
                        .as_str()
                        .ok_or("library has no id")?
                        .to_owned(),
                    krate: libraries[owner].krate.clone(),
                }
                .to_string());
            }
        }
        for (index, injection) in parsed.inject.iter().filter(|i| is_mod(&i.at)).enumerate() {
            let owner = libraries
                .iter()
                .position(|p| p.owns(&injection.at))
                .ok_or_else(|| {
                    format!(
                        "{}: {} has no library in this wasm dependency group",
                        injection.id, injection.at
                    )
                })?;
            let library = &members[owner];
            let id = library.manifest["id"].as_str().ok_or("library has no id")?;
            if !parsed.dependencies.contains_key(id) {
                return Err(format!(
                    "{}: mod point library {id} is not a declared dependency",
                    injection.id
                ));
            }
            let routed = mod_points::route(
                &parsed.id,
                injection,
                &mod_points::Library {
                    id,
                    version: library.manifest["version"]
                        .as_str()
                        .ok_or("library has no version")?,
                    catalog: &libraries[owner],
                },
            )
            .map_err(|e| e.to_string())?;
            let access = if routed.access == Access::Api {
                "listeners"
            } else {
                "raw"
            };
            let listener = if routed.access == Access::Api && injection.kind == Kind::Replace {
                format!(
                    "mod_api::replacement_listener!({}, none, crate::{})",
                    routed.point.returns, injection.with
                )
            } else {
                format!("crate::{}", injection.with)
            };
            let refused = index + 1;
            registration.push_str(&format!("if !{}::__horizon_points::register_{}_{}({:?}, {}, {:?}, {}, {}) {{ return {refused}; }}\n", library.krate, access, routed.place, parsed.id, injection.priority, injection.id, injection.kind == Kind::Replace, listener));
        }
        // Engine listeners keep their existing generated codecs and registration path.
        if let Some(array) = manifest
            .get_mut("inject")
            .and_then(|v| v.as_array_of_tables_mut())
        {
            array.retain(|i| {
                i.get("at")
                    .and_then(|v| v.as_str())
                    .is_some_and(|at| !is_mod(at))
            });
        }
        let (registrations, names) = registrations(&manifest, catalog)?;
        members[index].registrations = registrations;
        members[index].points = names;
        output.push(format!("\n#[cfg(target_arch = \"wasm32\")]\n#[unsafe(no_mangle)]\npub fn horizon_mod_points_register() -> u32 {{ {registration} 0 }}\n#[cfg(target_arch = \"wasm32\")]\n#[unsafe(no_mangle)]\npub fn horizon_mod_points_seal() {{ crate::__horizon_points::seal(); }}\n"));
    }
    Ok(output)
}
