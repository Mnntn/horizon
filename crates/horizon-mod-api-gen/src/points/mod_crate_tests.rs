use super::*;
use point_shape::catalog::Point;
use quote::ToTokens;

/// A mod crate `chemlib` in a fresh folder, with `lib.rs` and `tanks.rs`.
fn crate_with(name: &str, lib: &str, tanks: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mod-points-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"chemlib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dir.join("src/lib.rs"), lib).unwrap();
    fs::write(dir.join("src/tanks.rs"), tanks).unwrap();
    dir
}

const LIB: &str = r#"
pub mod tanks;
pub use tanks::Tank;
#[derive(mod_api::Wire)]
pub struct Chemical { pub id: u32 }
pub struct Opaque;
#[derive(mod_api::Wire)]
struct Hidden;
pub fn transfer(from: &mut u32, to: &mut u32, chemical: Chemical) {}
pub fn fill(tank: &mut Tank) {}
fn private_helper(amount: u32) -> u32 { amount }
pub fn opaque(value: Opaque) {}
fn hidden(value: Hidden) {}
mod vault { #[derive(mod_api::Wire)] pub struct Sealed; }
pub fn seal(value: vault::Sealed) {}
#[mod_api::no_point]
pub fn opted_out(amount: u32) {}
#[no_point]
fn opted_out_short() {}
#[cfg(feature = "x")]
pub fn gated() {}
pub const fn constant() -> u32 { 1 }
pub trait Reactor { fn react(&self); }
impl Reactor for Opaque { fn react(&self) {} }
"#;

const TANKS: &str = r#"
use crate::Chemical;
#[derive(Clone, mod_api::Wire)]
pub struct Tank { pub amount: u32 }
impl Tank {
    pub fn fill(&mut self, chemical: &Chemical, amount: u32) -> u32 { amount }
}
pub fn drain(tank: &mut Tank) {}
"#;

fn by_name<'a>(points: &'a [Point], name: &str) -> &'a Point {
    points
        .iter()
        .find(|point| point.name == name)
        .unwrap_or_else(|| panic!("no point {name}"))
}

#[test]
fn every_eligible_function_public_and_private_gets_a_point_by_the_engine_rules() {
    let dir = crate_with("list", LIB, TANKS);
    let catalog = mod_points(&dir, &WireTypes::new()).unwrap();
    assert_eq!(catalog.krate, "chemlib");
    assert_eq!(
        catalog.opted_out,
        ["chemlib::opted_out", "chemlib::opted_out_short"]
    );
    let points = catalog.points;
    let names: Vec<_> = points.iter().map(|point| point.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "fill",
            "hidden",
            "opaque",
            "private_helper",
            "seal",
            "tanks_drain",
            "tanks_tank_fill",
            "transfer"
        ],
        "opted-out, cfg-gated, const and trait-impl functions get none"
    );
    let transfer = by_name(&points, "transfer");
    assert_eq!(transfer.target, "chemlib::transfer");
    assert_eq!(transfer.api_refusal, None, "a pub derived type crosses");
    assert_eq!(transfer.returns, "cancel");
    assert!(transfer.raw);
    assert_eq!(
        transfer.listener.as_deref(),
        Some("crate::__TransferListener")
    );
    // `use crate::Chemical` resolves through the file's imports.
    let fill = by_name(&points, "tanks_tank_fill");
    assert_eq!(fill.target, "chemlib::tanks::Tank::fill");
    assert_eq!(
        fill.api_refusal.as_deref(),
        Some("its `self` has no API handle yet"),
        "a mod type's receiver is internals only, as on the engine"
    );
    assert_eq!(
        fill.listener.as_deref(),
        Some("crate::tanks::__TanksTankFillListener")
    );
    assert_eq!(
        by_name(&points, "opaque").api_refusal.as_deref(),
        Some("argument 1 `value` (`Opaque`) has no API type yet"),
        "a type without the derive is refused, naming it: nothing adds the derive"
    );
    assert_eq!(
        by_name(&points, "hidden").api_refusal.as_deref(),
        Some("argument 1 `value` (`Hidden`) has no API type yet"),
        "a private type can't be in a record addons read"
    );
    assert_eq!(
        by_name(&points, "seal").api_refusal.as_deref(),
        Some("argument 1 `value` (`vault::Sealed`) has no API type yet"),
        "a pub type no addon can reach, in a private module, is refused too"
    );
    assert_eq!(by_name(&points, "private_helper").returns, "result");
    assert_eq!(
        by_name(&points, "fill").api_refusal,
        None,
        "a re-exported child module's type resolves relative to the module"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_hash_changes_with_the_signature_and_only_with_it() {
    let hash = |name: &str, lib: &str| {
        let dir = crate_with(name, lib, TANKS);
        let points = mod_points(&dir, &WireTypes::new()).unwrap().points;
        let _ = fs::remove_dir_all(dir);
        by_name(&points, "transfer").hash.clone()
    };
    let base = hash("hash-base", LIB);
    assert_eq!(base.len(), 16);
    assert_eq!(
        base,
        hash(
            "hash-body",
            &LIB.replace(
                "chemical: Chemical) {}",
                "chemical: Chemical) { let _ = 1; }"
            )
        ),
        "the body is not the signature"
    );
    for (case, changed) in [
        ("hash-param", LIB.replace("to: &mut u32", "to: &mut u64")),
        ("hash-name", LIB.replace("to: &mut u32", "into: &mut u32")),
        (
            "hash-return",
            LIB.replace(
                "chemical: Chemical) {}",
                "chemical: Chemical) -> bool { true }",
            ),
        ),
        ("hash-ref", LIB.replace("from: &mut u32", "from: &u32")),
    ] {
        assert_ne!(base, hash(case, &changed), "{case}");
    }
}

#[test]
fn two_points_with_one_name_fail_the_build_naming_both() {
    let dir = crate_with(
        "clash",
        "pub mod tanks;\npub fn tanks_drain() {}\n",
        "pub fn drain() {}\n",
    );
    let error = mod_points(&dir, &WireTypes::new()).unwrap_err();
    assert!(
        error.contains("chemlib::tanks_drain") && error.contains("chemlib::tanks::drain"),
        "{error}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Snake-casing a path is not injective: `a::b_c` and `a_b::c` are both `a_b_c`. The build
/// refuses them, naming both, and nothing is refused when they differ.
#[test]
fn paths_that_snake_case_to_one_name_fail_the_build_naming_both() {
    let dir = crate_with(
        "clash-paths",
        "pub mod a;
pub mod a_b;
",
        "",
    );
    fs::write(
        dir.join("src/a.rs"),
        "pub fn b_c() {}
",
    )
    .unwrap();
    fs::write(
        dir.join("src/a_b.rs"),
        "pub fn c() {}
",
    )
    .unwrap();
    let error = mod_points(&dir, &WireTypes::new()).unwrap_err();
    assert!(
        error.contains("chemlib::a::b_c")
            && error.contains("chemlib::a_b::c")
            && error.contains("a_b_c"),
        "{error}"
    );
    fs::write(
        dir.join("src/a_b.rs"),
        "pub fn d() {}
",
    )
    .unwrap();
    assert!(mod_points(&dir, &WireTypes::new()).is_ok());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn staging_marks_each_point_and_writes_the_table_beside_the_crate_root() {
    let dir = crate_with("stage", LIB, TANKS);
    let points = stage_mod(&dir, &WireTypes::new()).unwrap().points;
    assert_eq!(points.len(), 8);
    let lib = fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert!(lib.contains(
        "#[::mod_api::__point(name = \"transfer\", table = \"crate::__horizon_points\", raw, free, api(Chemical = \"crate::Chemical\"))] pub fn transfer"
    ));
    assert!(lib.contains("pub struct __TransferRaw"));
    assert!(lib.ends_with("#[path = \"__horizon_points.rs\"]\npub mod __horizon_points;\n"));
    assert!(!lib.contains("__point(name = \"opted_out"));
    let tanks = fs::read_to_string(dir.join("src/tanks.rs")).unwrap();
    assert!(tanks.contains("#[::mod_api::__point(name = \"tanks_tank_fill\""));
    let table = fs::read_to_string(dir.join("src/__horizon_points.rs")).unwrap();
    let table = syn::parse_file(&table).expect("the table parses");
    let text = table.to_token_stream().to_string();
    for item in [
        "pub struct TransferArgs",
        "pub fn transfer (",
        "pub const fn tanks_tank_fill (id : & 'static str , listener : crate :: tanks :: __TanksTankFillListener)",
        "pub static TABLE",
    ] {
        assert!(text.contains(item), "{item}");
    }
    // Mod points mirror the engine's rule for functions without a `Game` receiver: their API
    // listeners get the record and no Host, on both targets (the staged table serves both).
    assert!(points.iter().all(|point| point.host == "none"));
    assert!(text.contains("PointHost :: None"));
    for host in [
        "host :",
        "ServerHost",
        "ClientHost",
        "HostSide",
        "PointHost :: Read",
    ] {
        assert!(
            !text.contains(host),
            "a mod point's listener takes no host: {host}"
        );
    }
    let _ = fs::remove_dir_all(dir);
}
