use super::*;
use crate::datapack::{self, DataPack, DataStore};
use horizon_mod_package::manifest::MissingJar;
use std::io::Write as _;

struct Fixture(PathBuf);
impl Fixture {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("horizon-jar-data-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let base = root.join("base");
        small_base(&base);
        fs::write(base.join("horizon-resources.json"), "{}").unwrap();
        fs::write(
            base.join("facts.json"),
            r#"{"states":[["minecraft:stone"]],"defaults":{"minecraft:stone":0}}"#,
        )
        .unwrap();
        fs::write(
            base.join("gameplay-facts.json"),
            r#"{"states":[{"item":"minecraft:stone"}]}"#,
        )
        .unwrap();
        Self(root)
    }
    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn jar(&self, id: &str, digit: char, files: &[(&str, &str)]) -> JarRef {
        // The launcher owns byte verification. These tests exercise the already
        // addressed ZIP inputs at the game-side composition boundary.
        let jar = JarRef::new(id.into(), "1".into(), digit.to_string().repeat(128)).unwrap();
        let path = jar.cache_path(&self.0.join("jars"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (path, text) in files {
            zip.start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        jar
    }
    fn content(&self, id: &str, files: &[(&str, &str)], jars: Vec<JarRef>) -> ContentMod {
        let text = manifest(id, "");
        let mut files = files.to_vec();
        files.push(("horizon-mod.toml", &text));
        let mut content = folder_mod(&self.0.join(id), &files);
        content.jars = jars;
        content
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn jar_data_layers_merge_tags_and_replace_recipes_below_world_datapacks() {
    let f = Fixture::new("order");
    let tag = "data/minecraft/tags/function/ordered.json";
    let recipe = "data/alpha/recipe/jar_probe.json";
    let json = |result| {
        format!(
            r#"{{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{{"id":"minecraft:{result}","count":1}}}}"#
        )
    };
    f.write(
        &format!("base/{tag}"),
        r#"{"values":["minecraft:layers/base"]}"#,
    );
    f.write(
        "base/data/minecraft/function/layers/base.mcfunction",
        "say base",
    );
    f.write(&format!("base/{recipe}"), json("stone"));
    let first_recipe = json("dirt");
    let second_recipe = json("sand");
    let own_recipe = json("stick");
    let later_recipe = json("diamond");
    let first = f.jar(
        "first",
        'a',
        &[
            (tag, r#"{"values":["minecraft:layers/first"]}"#),
            (recipe, &first_recipe),
            (
                "data/minecraft/function/layers/first.mcfunction",
                "say first",
            ),
            (
                "data/original/function/passive.mcfunction",
                "say original namespace",
            ),
            ("data/../../outside.json", "not a data path"),
            ("Example.class", "not copied"),
            ("pack.mcmeta", "not pack metadata"),
        ],
    );
    let second = f.jar(
        "second",
        'b',
        &[
            (tag, r#"{"values":["minecraft:layers/second"]}"#),
            (recipe, &second_recipe),
            (
                "data/minecraft/function/layers/second.mcfunction",
                "say second",
            ),
        ],
    );
    let later = f.jar(
        "later",
        'c',
        &[
            (tag, r#"{"values":["minecraft:layers/later"]}"#),
            (recipe, &later_recipe),
            (
                "data/minecraft/function/layers/later.mcfunction",
                "say later",
            ),
        ],
    );
    let alpha = f.content(
        "alpha",
        &[
            (tag, r#"{"values":["alpha:layers/own"]}"#),
            (recipe, &own_recipe),
            ("data/alpha/function/layers/own.mcfunction", "say own"),
        ],
        vec![first, second],
    );
    let beta = f.content("beta", &[], vec![later]);
    let root = f.0.join("root");
    let mods = [alpha, beta];
    build_fixture(&f.0.join("base"), &f.0.join("jars"), &mods, &root).unwrap();
    let resources = resolved_resources(
        &f.0.join("base"),
        root.clone(),
        &f.0.join("jars"),
        &mods,
        f.0.join("cache"),
    )
    .unwrap();
    let values: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(tag)).unwrap()).unwrap();
    assert_eq!(
        values["values"],
        serde_json::json!([
            "minecraft:layers/base",
            "minecraft:layers/first",
            "minecraft:layers/second",
            "alpha:layers/own",
            "minecraft:layers/later"
        ])
    );
    assert_eq!(
        resources.data().read(recipe).unwrap(),
        later_recipe.as_bytes()
    );
    assert!(
        resources
            .data()
            .contains("data/original/function/passive.mcfunction")
    );
    assert!(
        !root
            .join("data/original/function/passive.mcfunction")
            .exists()
    );
    assert_eq!(
        fs::read(root.join(recipe)).unwrap(),
        json("stone").as_bytes()
    );
    let namespaces: Vec<String> =
        serde_json::from_slice(&fs::read(root.join(MARKER)).unwrap()).unwrap();
    assert!(namespaces.iter().any(|ns| ns == "original"));
    assert!(!root.join("Example.class").exists());
    assert!(!root.join("outside.json").exists());
    assert!(!f.0.join("outside.json").exists());

    f.write(
        "world/datapacks/overrides/pack.mcmeta",
        r#"{"pack":{"pack_format":1,"description":"test"}}"#,
    );
    let world_recipe = json("emerald");
    f.write(
        &format!("world/datapacks/overrides/{recipe}"),
        &world_recipe,
    );
    f.write(
        &format!("world/datapacks/overrides/{tag}"),
        r#"{"values":["minecraft:layers/world"]}"#,
    );
    f.write(
        "world/datapacks/overrides/data/minecraft/function/layers/world.mcfunction",
        "say world",
    );
    let builtin = DataPack::from_source("vanilla".into(), resources.data().clone());
    let world = datapack::discover(&f.0.join("world"));
    let mut packs = vec![&builtin];
    packs.extend(world.iter());
    let store = DataStore::load(&packs);
    assert!(store.errors.is_empty(), "{:?}", store.errors);
    assert_eq!(
        store.function_tags["minecraft:ordered"]
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        [
            "minecraft:layers/base",
            "minecraft:layers/first",
            "minecraft:layers/second",
            "alpha:layers/own",
            "minecraft:layers/later",
            "minecraft:layers/world"
        ]
    );
    // The real server data store supplies these as overrides above the
    // retained passive recipe, which still contains the later mod's data.
    assert_eq!(
        store.files["recipe/alpha:jar_probe"].last().unwrap(),
        world_recipe.as_bytes()
    );
}

#[test]
fn changing_jar_list_or_order_rebuilds_and_missing_after_compose_keeps_the_old_root() {
    let f = Fixture::new("stamp");
    let path = "assets/original/value.png";
    let a = f.jar("first", 'a', &[(path, "a")]);
    let b = f.jar("second", 'b', &[(path, "b")]);
    let mut mods = vec![f.content("alpha", &[], vec![a.clone(), b.clone()])];
    let base = f.0.join("base");
    let store = f.0.join("jars");
    let cache = f.0.join("cache");
    let first = resource_root_fixture(&base, &store, &mods, &cache).unwrap();
    let first_source = resolved_resources(&base, first.clone(), &store, &mods, cache.clone())
        .unwrap()
        .source()
        .clone();
    assert_eq!(first_source.read(path).unwrap(), b"b");
    assert_eq!(
        resource_root_fixture(&base, &store, &mods, &cache).unwrap(),
        first
    );
    mods[0].jars.reverse();
    let reversed = resource_root_fixture(&base, &store, &mods, &cache).unwrap();
    let reversed_source = resolved_resources(&base, reversed.clone(), &store, &mods, cache.clone())
        .unwrap()
        .source()
        .clone();
    assert_ne!(first, reversed);
    assert_eq!(reversed_source.read(path).unwrap(), b"a");
    mods[0].jars.pop();
    let shortened = resource_root_fixture(&base, &store, &mods, &cache).unwrap();
    let shortened_source =
        resolved_resources(&base, shortened.clone(), &store, &mods, cache.clone())
            .unwrap()
            .source()
            .clone();
    assert_ne!(shortened, reversed);
    assert_eq!(shortened_source.read(path).unwrap(), b"b");
    check_jars(&shortened, &store).unwrap();
    let relabeled = JarRef::new(b.id().into(), "2".into(), b.sha512().into()).unwrap();
    mods[0].jars[0] = relabeled.clone();
    let versioned = resource_root_fixture(&base, &store, &mods, &cache).unwrap();
    assert_ne!(versioned, shortened);
    let accepted: Vec<horizon_mod_package::manifest::JarRequirement> =
        serde_json::from_slice(&fs::read(versioned.join(JAR_INPUTS)).unwrap()).unwrap();
    assert_eq!(accepted[0].jar, relabeled);
    mods[0].jars[0] = b.clone();
    assert_eq!(
        resource_root_fixture(&base, &store, &mods, &cache).unwrap(),
        shortened
    );
    fs::remove_file(b.cache_path(&store)).unwrap();
    let before = fs::read_dir(&cache).unwrap().count();
    let error = resource_root_fixture(&base, &store, &mods, &cache).unwrap_err();
    let missing = error.downcast_ref::<MissingJar>().unwrap();
    assert_eq!(missing.owners, ["alpha"]);
    assert_eq!(missing.jar, b);
    assert_eq!(missing.path, b.cache_path(&store));
    assert_eq!(fs::read_dir(&cache).unwrap().count(), before);
    assert_eq!(shortened_source.read(path).unwrap(), b"b");
    let error = check_jars(&shortened, &store).unwrap_err();
    let JarInputsError::Missing(accepted_missing) = error else {
        panic!("expected typed missing accepted input, got {error:?}");
    };
    assert_eq!(accepted_missing.as_ref(), missing);
    // Neither failed preparation mutates any previously published snapshot.
    assert_eq!(first_source.read(path).unwrap(), b"b");
    assert_eq!(reversed_source.read(path).unwrap(), b"a");
}

#[test]
fn accepted_jar_inputs_distinguish_unreadable_invalid_and_non_file_cache_entries() {
    let fixture = Fixture::new("accepted-input-errors");
    let root = fixture.0.join("accepted");
    let store = fixture.0.join("jars");
    fixture.write(&format!("accepted/{MARKER}"), "{}");
    let path = root.join(JAR_INPUTS);
    let error = check_jars(&root, &store).unwrap_err();
    assert!(matches!(
        error,
        JarInputsError::Io { path: failed, source }
            if failed == path && source.kind() == std::io::ErrorKind::NotFound
    ));

    fixture.write(&format!("accepted/{JAR_INPUTS}"), "[");
    assert!(matches!(
        check_jars(&root, &store).unwrap_err(),
        JarInputsError::Invalid { path: failed, .. } if failed == path
    ));

    let jar = JarRef::new("original".into(), "1".into(), "d".repeat(128)).unwrap();
    let requirements = [horizon_mod_package::manifest::JarRequirement {
        owners: vec!["alpha".into(), "beta".into()],
        jar: jar.clone(),
    }];
    fixture.write(
        &format!("accepted/{JAR_INPUTS}"),
        serde_json::to_vec(&requirements).unwrap(),
    );
    fs::create_dir_all(jar.cache_path(&store)).unwrap();
    assert!(matches!(
        check_jars(&root, &store).unwrap_err(),
        JarInputsError::Io { path: failed, source }
            if failed == store && source.kind() == std::io::ErrorKind::InvalidInput
    ));
}

#[test]
fn code_only_inventory_changes_cache_roots_and_recipe_conditions_without_content_namespaces() {
    use horizon_mod_package::{Side, enabled::EnabledMods};
    let fixture = Fixture::new("enabled-inventory");
    fixture.write(
        "base/data/minecraft/recipe/code_only.json",
        r#"{"type":"test:recipe","neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"code_only"}]}"#,
    );
    fixture.write(
        "base/data/minecraft/recipe/namespace_is_not_a_mod.json",
        r#"{"type":"test:recipe","neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"unrelated_namespace"}]}"#,
    );
    fixture.write("base/assets/unrelated_namespace/lang/en_us.json", "{}");
    let base = fixture.0.join("base");
    let jars = fixture.0.join("jars");
    let cache = fixture.0.join("cache");
    let both = EnabledMods::new([("code_only".into(), Side::Both)]);
    let first = super::super::resource_root(&base, &jars, &[], &cache, &both).unwrap();
    assert_ne!(first, base);
    assert_eq!(EnabledMods::read(&first).unwrap(), both);
    assert_eq!(
        super::super::resource_root(&base, &jars, &[], &cache, &both).unwrap(),
        first
    );
    let recipes = |root: &Path| {
        horizon_items::recipe_sources::sources(root, &Default::default())
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>()
    };
    assert_eq!(recipes(&first), ["code_only"]);
    let client = EnabledMods::new([("code_only".into(), Side::Client)]);
    let next = super::super::resource_root(&base, &jars, &[], &cache, &client).unwrap();
    assert_ne!(next, first);
    assert!(recipes(&next).is_empty());
    let disabled =
        super::super::resource_root(&base, &jars, &[], &cache, &EnabledMods::default()).unwrap();
    assert_eq!(disabled, base);
    assert!(recipes(&disabled).is_empty());
    // A reload creates another generation; readers of the old root retain
    // the inventory and conditions they were constructed with.
    assert_eq!(recipes(&first), ["code_only"]);
}

/// A biome modifier a code mod declares applies over the content root with no data pack at
/// all, and a root whose mods ship no modifier file still keeps its biomes as they were.
#[test]
fn a_biome_modifier_declared_in_code_applies_to_the_root_biomes() {
    let base = biome_base("biome-modifiers-declared");
    let scratch = base.parent().unwrap().to_owned();
    let m = zinc_mod(&scratch.join("zinc"), &[]);
    let out = scratch.join("out");
    build_fixture(&base, &base.join("jars"), &[m], &out).unwrap();
    // No file modifier: the biome bundle is the base's, byte for byte.
    assert_eq!(
        fs::read(out.join("worldgen/biomes.json")).unwrap(),
        fs::read(base.join("worldgen/biomes.json")).unwrap()
    );
    let packs = horizon_world::worldgen::overrides::WorldgenOverrides::default();
    assert!(
        biome_modifiers_over_packs(&out, &packs, &[])
            .unwrap()
            .is_none()
    );
    let declared = mod_api::BiomeModifier {
        biomes: vec!["#minecraft:is_overworld".to_owned()],
        operation: mod_api::BiomeModifierOperation::AddFeatures {
            features: vec!["zinc:ore_zinc".to_owned()],
            step: "underground_ores".to_owned(),
        },
    };
    let declared = [[
        "zinc".to_owned(),
        "data/zinc/neoforge/biome_modifier/declared.json".to_owned(),
        declared.json(),
    ]];
    let after = biome_modifiers_over_packs(&out, &packs, &declared)
        .unwrap()
        .expect("a declared modifier changes the biomes");
    let file_mod = zinc_mod(&scratch.join("file-zinc"), &[("declared", &declared[0][2])]);
    let file_out = scratch.join("file-out");
    build_fixture(&base, &base.join("jars"), &[file_mod], &file_out).unwrap();
    let original: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(out.join("worldgen/biomes.json")).unwrap()).unwrap();
    let expected: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(file_out.join("worldgen/biomes.json")).unwrap()).unwrap();
    assert_eq!(after.apply("biomes", original), expected);
    let plains = after
        .entries("biomes")
        .find(|(id, _)| *id == "minecraft:plains")
        .unwrap()
        .1;
    let plains: serde_json::Value = serde_json::from_str(plains).unwrap();
    assert_eq!(
        plains["features"],
        serde_json::json!([
            [],
            ["minecraft:lake"],
            [],
            [],
            [],
            [],
            ["minecraft:ore_coal", "zinc:ore_zinc"]
        ])
    );
    // The nether is not in the overworld tag: untouched, so not written out.
    assert!(
        after
            .entries("biomes")
            .all(|(id, _)| id != "minecraft:nether_wastes")
    );
    fs::remove_dir_all(scratch).unwrap();
}
