//! Production records and callers, through the real sealed subscription registry.
use super::*;
use crate::native_mods::{api, internals::Catalog, slots};
use horizon_mod_package::{
    manifest::inject::{Access, Inject, Kind, SlotKind},
    Manifest,
};
use std::{fs, process::Command};

#[test]
#[ignore = "run tools/ci/listed_calls.py to export production records first"]
fn production_port_listeners_attach_and_invoke() {
    let input = std::env::var_os("HORIZON_LISTED_PROOF_OUT").expect("run tools/ci/listed_calls.py");
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    assert_eq!(rows.len(), 4);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let dir = root.join("target/listed-port-loader");
    fs::create_dir_all(dir.join("src")).unwrap();
    crate::native_mods::tests::write_bundle(&dir, "example", "");
    let base = fs::read_to_string(dir.join(horizon_mod_package::manifest::MANIFEST)).unwrap();
    fs::write(dir.join("Cargo.toml"), format!("[package]\nname='listed-port-loader-proof'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\nmod_points={{package='horizon-mod-points',path={:?}}}\nhorizon-mod-api-macros={{path={:?}}}\n", root.join("crates/horizon-mod-points"), root.join("crates/horizon-mod-api-macros"))).unwrap();
    for row in rows {
        let original: Site = serde_json::from_value(row["site"].clone()).unwrap();
        let enclosing = row["enclosing"].clone();
        let name = &original.name;
        let canonical = original.export();
        let caller = row["caller"].as_str().unwrap();
        let (args, callback, driver) = if original.once {
            (
                "Args",
                "args.stack.0 += 1;",
                format!(
                    r#"
                impl crate::game::Game<'_> {{
                    fn dispense_item(&mut self, _: (i32,i32,i32), _: crate::geometry::Direction, stack: crate::items::ItemStack) -> __{name}Output {{ Ok(Some(stack)) }}
                    fn probe(&mut self, key: (i32,i32,i32), facing: crate::geometry::Direction, stack: crate::items::ItemStack) -> __{name}Output {{ {caller} }}
                }}
                pub fn invoke(enabled: bool) {{
                    let mut state = 0; let mut receiver = crate::game::Game(&mut state);
                    let result = receiver.probe((1,2,3), crate::geometry::Direction, crate::items::ItemStack(41)).unwrap().unwrap();
                    assert_eq!(result.0, if enabled {{42}} else {{41}});
                }}
            "#
                ),
            )
        } else if original.call.ends_with("farmland") {
            (
                "Args<'_, '_>",
                "assert_eq!(*args.w.value, 7); args.p[0] += 1;",
                format!(
                    r#"
                fn farmland(w: &crate::mobs::MobWorld<'_>, p: [i32;3]) -> bool {{ assert_eq!(*w.value, 7); p[0] == 42 }}
                fn probe(w: &crate::mobs::MobWorld<'_>, p: [i32;3]) -> bool {{ {caller} }}
                pub fn invoke(enabled: bool) {{
                    let value = 7; let world = crate::mobs::MobWorld {{ value: &value }};
                    assert_eq!(probe(&world, [41,0,0]), enabled);
                }}
            "#
                ),
            )
        } else {
            let folder = if original.ordinal == 0 {
                "hanging_signs"
            } else {
                "signs"
            };
            (
                "Args<'_>",
                "args.wood = \"changed\";",
                format!(
                    r#"
                fn editor_sprite(folder: &str, wood: &str) -> String {{ format!("{{folder}}:{{wood}}") }}
                fn probe(wood: &str) -> String {{ {caller} }}
                pub fn invoke(enabled: bool) {{
                    let text = String::from("oak");
                    assert_eq!(probe(&text), format!("{folder}:{{}}", if enabled {{"changed"}} else {{"oak"}}));
                    assert_eq!(text, "oak");
                }}
            "#
                ),
            )
        };
        let callback = format!(
            "use crate::mod_0_internals::raw::{{__{canonical}Args as Args, __{canonical}Receiver as Receiver}}; pub fn hook(_: Receiver<'_, '_>, args: &mut {args}) {{ crate::CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); {callback} }}"
        );
        let site = original.clone();
        let mut manifest = Manifest::parse(&base).unwrap();
        manifest.inject.push(Inject {
            id: "port".into(),
            at: site.target.clone(),
            kind: Kind::BeforeCall,
            with: "hook".into(),
            access: Access::Internals,
            priority: 0,
            hash: Some(enclosing["hash"].as_str().unwrap().into()),
            call: Some(site.call.clone()),
            nth: Some(site.ordinal),
            ctx: Some(site.ctx.clone()),
        });
        fs::write(
            dir.join("internals.json"),
            serde_json::to_vec(&serde_json::json!([enclosing])).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("points.json"),
            serde_json::to_vec(&serde_json::json!({"points": [], "sites": [row["site"]]})).unwrap(),
        )
        .unwrap();
        let catalog = Catalog::read(&dir).unwrap();
        assert_eq!(
            slots::routes(&catalog, &manifest).unwrap(),
            [SlotKind::Site]
        );
        if site.target.contains("::game::") {
            let mut nominal = manifest.clone();
            nominal.inject[0].at = site
                .target
                .replace("::sign_edit", "")
                .replace("::machines", "");
            assert!(matches!(
                slots::routes(&catalog, &nominal),
                Err(horizon_mod_package::manifest::inject::InjectError::Missing { .. })
            ));
        }
        let registration = api::main_rs("/* MOD_SLOTS */", &[&manifest], &catalog);
        let (mut exports, mut constructors, mut crates) = (
            std::collections::BTreeSet::new(),
            std::collections::BTreeSet::new(),
            std::collections::BTreeSet::new(),
        );
        site.facade(&mut exports, &mut constructors, &mut crates);
        let mut module = format!("{}\n{driver}", row["raw"].as_str().unwrap());
        for segment in site
            .listener
            .rsplit_once("::")
            .unwrap()
            .0
            .split("::")
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            module = format!("pub mod {segment} {{ {module} }}");
        }
        // Only engine values/callees are stand-ins. Records, actual generated caller,
        // facade, adapter, registry, subscription flag and dispatch macro are real.
        let module = module
            .replace(
                "pub mod game {",
                "pub mod game { pub struct Game<'g>(pub &'g mut usize);",
            )
            .replace(
                "pub mod mobs {",
                "pub mod mobs { pub struct MobWorld<'g> { pub value: &'g usize }",
            );
        let aliases = if site.call.ends_with("farmland") {
            "pub mod game { pub struct Game<'g>(pub &'g mut usize); }"
        } else {
            "pub mod mobs {}"
        };
        let invoke_path = site.listener.rsplit_once("::").unwrap().0;
        let program = format!(
            r#"
extern crate self as horizon_client; extern crate self as horizon_client_lib;
extern crate self as horizon_client_render; extern crate self as horizon_core;
extern crate self as horizon_items;
pub mod geometry {{ pub struct Direction; }} pub mod items {{ pub struct ItemStack(pub usize); }}
static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
{aliases}
{module}
mod mod_0_internals {{ pub mod raw {{ {exports} {constructors} }} }}
mod mod_0 {{ {callback} }}
fn main() {{
use std::sync::atomic::Ordering::Relaxed;
let site = mod_points::SiteId::{name};
assert!(!mod_points::__private::TABLE.active_site(site as usize));
{invoke_path}::invoke(false); assert_eq!(CALLS.load(Relaxed), 0);
let listeners = [{registration}];
for listener in &listeners {{ mod_points::register(listener).unwrap(); }}
mod_points::__private::seal();
assert!(mod_points::__private::TABLE.active_site(site as usize));
assert!(!mod_points::__private::TABLE.active_site(mod_points::SiteId::S7173cf1fd760a6e4 as usize));
{invoke_path}::invoke(true); assert_eq!(CALLS.load(Relaxed), 1);
}}
"#,
            exports = exports.into_iter().collect::<String>(),
            constructors = constructors.into_iter().collect::<String>()
        );
        fs::write(dir.join("src/main.rs"), program).unwrap();
        let output = Command::new("cargo")
            .args(["run", "--offline", "--quiet", "--manifest-path"])
            .arg(dir.join("Cargo.toml"))
            .env(
                "CARGO_TARGET_DIR",
                root.join("target/listed-port-loader-target"),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            site.call,
            String::from_utf8_lossy(&output.stderr)
        );
        let mut stale = manifest.clone();
        stale.inject[0].hash = Some("stale".into());
        let error = slots::routes(&catalog, &stale).unwrap_err().to_string();
        assert!(error.contains(&site.target), "{error}");
    }
    fs::remove_dir_all(dir).unwrap();
}
