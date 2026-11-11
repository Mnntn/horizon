pub trait Hooks {
    /// Register exact cauldron item interactions on both sides before snapshot publication.
    /// Vanilla dye tags take precedence, and duplicate entries refuse preparation.
    #[notes = "## Cauldron interactions

`register_cauldron_interaction` registers exact item callbacks on both sides. See [Registering cauldron interactions](../how-to/native-mods.md#registering-cauldron-interactions) for the read-only decision, transactional fire, duplicate refusal and server-corrected prediction contract."]
    #[hook(
        side = "both",
        contract = "listen",
        context = "registration",
        neoforge = "RegisterCauldronInteractionEvent"
    )]
    fn register_cauldron_interaction();
}
