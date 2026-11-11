//! The `living_death_message` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `native_living_death_message(entity, kind, message)` is asked for the death message a tamed mob sends its owner (`TamableAnimal.die`), once the message is built and only when it would be sent (owner online, `show_death_messages` on). `message` holds the text: replace it, or set it to `None` to tell the owner nothing. Every mod is asked in order. Server and host. Bundles need "hooks": @API:living_death_message@.
    #[hook(
        side = "server",
        contract = "collect",
        out = "message",
        context = "loop",
        target = "horizon_client_lib::living_death::native_living_death_message"
    )]
    fn living_death_message();
}
