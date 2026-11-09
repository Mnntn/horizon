//! The energy capability (arismoko/horizon#41 §7): [`ENERGY`], as NeoForge 26.3's
//! `Capabilities.Energy.BLOCK`, an [`EnergyHandler`]: an amount and a capacity, with no resource,
//! written only inside a transaction.
use crate::content::resource::{Ends, move_once};
use crate::{
    BlockSide, Capability, CapabilityKey, ItemCapabilityKey, Overanswered, ServerHost, Transaction,
};
use std::marker::PhantomData;

/// A block's energy (NeoForge's `EnergyHandler`): stateless, given the host, and written only in
/// a transaction, as a [`ResourceHandler`](crate::ResourceHandler) is.
pub trait EnergyHandler: Send + Sync {
    /// How much it holds.
    fn amount(&self, host: &mut dyn ServerHost) -> u32;
    /// How much it holds when full.
    fn capacity(&self, host: &mut dyn ServerHost) -> u32;
    /// Puts up to `amount` in, in transaction `tx`: how much went. Never more than `amount`.
    fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32;
    /// Takes up to `amount` out, in transaction `tx`: how much came. Never more than `amount`.
    fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32;
}

/// Energy, as NeoForge 26.3's `Capabilities.Energy.BLOCK`: a block entity's energy as one side
/// reaches it. The game answers it for a mod block entity's buffer (`block_entity_energy`, as
/// `SimpleEnergyHandler`): its capacity, the most one insert and one extract move, and the sides
/// a fill comes through and a drain leaves by.
pub const ENERGY: CapabilityKey<dyn EnergyHandler> = CapabilityKey {
    name: "horizon:energy",
    default: Some(|host, at| {
        crate::backend_of(host)
            .and_then(|backend| backend.energy_amount(at))
            .map(|_| Box::new(GameEnergy(at)) as Box<dyn EnergyHandler>)
    }),
    handler: PhantomData,
};

/// Energy held by an item, as NeoForge 26.3's `Capabilities.Energy.ITEM`: a handler for the item at
/// one place, found with [`ItemPlace::capability`](crate::ItemPlace::capability), whose changes
/// rewrite the item there with an [`exchange`](crate::ItemPlace::exchange) of one, so from a stack
/// of more than one the changed item splits off. Vanilla has no energy items, so only mods answer
/// it.
pub const ITEM_ENERGY: ItemCapabilityKey<dyn EnergyHandler> =
    ItemCapabilityKey::new("horizon:item_energy");

/// The game's own [`ENERGY`]: a mod block entity's buffer, as `.side` reaches it.
struct GameEnergy(BlockSide);
impl EnergyHandler for GameEnergy {
    fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        crate::backend_of(host)
            .and_then(|backend| backend.energy_amount(self.0))
            .unwrap_or(0)
    }
    fn capacity(&self, host: &mut dyn ServerHost) -> u32 {
        crate::backend_of(host).map_or(0, |backend| backend.energy_capacity(self.0))
    }
    fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        crate::backend_mut_of(host)
            .map_or(0, |backend| backend.energy_insert(self.0, amount, tx))
            .min(amount)
    }
    fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
        crate::backend_mut_of(host)
            .map_or(0, |backend| backend.energy_extract(self.0, amount, tx))
            .min(amount)
    }
}

/// Moves up to `max` energy out of `from` into `to` (NeoForge's `EnergyHandlerUtil.move`), in
/// transaction `tx`: how much moved. As one step of `move_resources`: all or nothing in a
/// transaction of its own, a move `to` takes only part of tried again with that part, and a
/// handler answering more than it was asked ending it with [`Overanswered`].
pub fn move_energy(
    host: &mut dyn ServerHost,
    from: &Capability<dyn EnergyHandler>,
    to: &Capability<dyn EnergyHandler>,
    max: u32,
    tx: &Transaction,
) -> Result<u32, Overanswered> {
    let _ = tx;
    let want = max.min(from.amount(host));
    if want == 0 {
        return Ok(0);
    }
    move_once(
        host,
        Ends {
            from,
            to,
            at: &"energy",
        },
        want,
        |host, tx, want| from.extract(host, tx, want),
        |host, tx, out| to.insert(host, tx, out),
    )
}
