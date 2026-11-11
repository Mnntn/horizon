use super::{read_lock, registry_lock::RegistryLock};

#[test]
fn generated_game_edges_are_frozen_before_mod_resolution() {
    let root = tempfile::tempdir().unwrap();
    let put = |name: &str, text: &str| {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    put(
        "crates/horizon-client/src/game.rs",
        "struct Game; impl Game { fn tick(&mut self) {} }",
    );
    put(
        "crates/horizon-client/Cargo.toml",
        "[package]\nname = 'horizon-client'\nversion = '0.1.0'\n",
    );
    put(
        "crates/horizon-client-input/src/lib.rs",
        "pub fn pressed() -> bool { true }",
    );
    put("crates/horizon-client-input/Cargo.toml", "[package]\nname = 'horizon-client-input'\nversion = '0.1.0'\n[dependencies]\nlog = '0.4'\n");
    put(
        "crates/horizon-mod-api-gen/slots/horizon.txt",
        "horizon_client_input::pressed\n",
    );
    put(
        "crates/horizon-mod-points/Cargo.toml",
        "[package]\nname = 'horizon-mod-points'\n",
    );
    let mut original = String::from("version = 4\n\n");
    for name in [
        "horizon-client",
        "horizon-client-input",
        "horizon-mod-api",
        "horizon-mod-api-macros",
        "horizon-mod-points",
    ] {
        original.push_str(&format!(
            "[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"
        ));
        if name == "horizon-client-input" {
            original.push_str("dependencies = [\n \"log\",\n]\n");
        }
        original.push('\n');
    }
    original.push_str("[[package]]\nname = \"log\"\nversion = \"0.4.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n");
    put("Cargo.lock", &original);
    horizon_platform::mod_build::api_gen::points::stage(root.path()).unwrap();
    let staged = read_lock(root.path()).unwrap();
    assert_ne!(original, staged);
    let plan = super::registry_plan::Plan::read(&staged, &[], &[]).unwrap();
    let game = &plan.game;
    // The old archive baseline rejects the actual generator's path edges.
    assert!(game
        .check_resolution(game, &RegistryLock::parse(&original).unwrap())
        .is_err());
    plan.allowed.check_resolution(game, game).unwrap();
    for changed in [
        staged.replace(" \"horizon-mod-api\",\n", ""),
        staged.replace(
            "checksum = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
            "checksum = \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"",
        ),
        staged.replace("version = \"0.4.0\"", "version = \"0.5.0\""),
    ] {
        assert_ne!(changed, staged);
        assert!(plan
            .allowed
            .check_resolution(&RegistryLock::parse(&changed).unwrap(), game)
            .is_err());
    }
}
