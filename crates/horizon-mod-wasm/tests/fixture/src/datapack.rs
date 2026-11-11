//! Assertions run inside the real wasm guest, not in a native stand-in.
pub fn read(host: &dyn mod_api::Host) {
    use mod_api::{DatapackReadError as E, DatapackValue, DimensionKey, LevelRef, WorldEpoch};
    let level = LevelRef::new(WorldEpoch::new(1), DimensionKey::OVERWORLD);
    assert_eq!(
        level.datapack_entries(host, "wasm_fixture:spell"),
        Some(vec!["addon:ice".into(), "wasm_fixture:fire".into()])
    );
    let handle = level
        .datapack_handle(host, "wasm_fixture:spell", "wasm_fixture:fire")
        .unwrap();
    assert_eq!(
        level.datapack_read(host, &handle),
        Ok(vec![("power".into(), DatapackValue::Int(7))])
    );
    assert_eq!(level.datapack_entries(host, "missing:registry"), None);
    assert_eq!(
        level.datapack_handle(host, "wasm_fixture:spell", "wasm_fixture:missing"),
        Err(E::MissingEntry)
    );
    assert!(host.is_loaded("datapack_fixture_completed"));
}
