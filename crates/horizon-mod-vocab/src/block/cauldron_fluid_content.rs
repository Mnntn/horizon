//! The fluid a cauldron block holds, registered for the fluid capability.
use crate::{BlockKey, FluidKey};

/// Registrar of the fluid a cauldron block holds (`register_cauldron_fluid_content`), as
/// NeoForge's `RegisterCauldronFluidContentEvent`.
pub trait CauldronFluidContents {
    /// `block` holds `total` millibuckets of `fluid` when full. `level` names the block's
    /// integer level property, whose greatest value is the number of levels the cauldron
    /// fills by; none means the cauldron is only ever full, as a lava cauldron. Each level
    /// holds `total` ÷ the number of levels, so `total` must divide evenly by it.
    /// Totals must be in `1..=i32::MAX`, the fluid capability amount range. The property
    /// must contain every canonical integer from 1 through its maximum (and may include 0).
    fn register(&mut self, block: BlockKey, fluid: FluidKey, total: u32, level: Option<&str>);
}

// no wire: engine-owned registration record, consumed on the host after the hook is collected.
/// One registration: what [`CauldronFluidContents::register`] was given.
// hidden: the engine resolves the registrations after collecting the hook; not part of the mod API
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CauldronFluidContent {
    /// The cauldron block.
    pub block: BlockKey,
    /// The fluid it holds.
    pub fluid: FluidKey,
    /// Millibuckets when full.
    pub total: u32,
    /// The name of the block's integer level property; none for a cauldron only ever full.
    pub level: Option<String>,
}

// no wire: engine-owned registrar backend; access stays scoped to the owning call.
/// Engine-owned storage behind the registrar.
#[derive(Default)]
#[non_exhaustive]
pub struct CauldronFluidContentRegistry {
    entries: Vec<CauldronFluidContent>,
}
impl CauldronFluidContentRegistry {
    /// Engine-owned entries, in registration order.
    // hidden: the engine consumes registrations after collecting the hook; not part of the mod API
    #[doc(hidden)]
    pub fn into_entries(self) -> Vec<CauldronFluidContent> {
        self.entries
    }
}
impl CauldronFluidContents for CauldronFluidContentRegistry {
    fn register(&mut self, block: BlockKey, fluid: FluidKey, total: u32, level: Option<&str>) {
        self.entries.push(CauldronFluidContent {
            block,
            fluid,
            total,
            level: level.map(str::to_owned),
        });
    }
}
