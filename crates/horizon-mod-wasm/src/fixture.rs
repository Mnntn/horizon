//! Real-runtime fixture access for downstream adapter tests; never enabled by game builds.
use crate::{WasmMod, link::Member, runtime};

/// Instantiate a synthetic shared-memory guest with the production runtime.
pub fn module(bytes: &[u8]) -> Result<WasmMod, String> {
    let engine = runtime::engine().map_err(|error| error.to_string())?;
    WasmMod::instantiate(
        &engine,
        &[],
        &[Member {
            name: "fixture",
            module: bytes,
        }],
        None,
    )
    .map_err(|error| format!("{error:?}"))?
    .into_iter()
    .next()
    .ok_or_else(|| "fixture exported no mod boundary".to_owned())
}

/// Resolve a production host row without pinning generated numeric IDs in a fixture.
pub fn import_id(owner: &str, name: &str) -> Option<u32> {
    crate::boundary::import_id(owner, name)
}
