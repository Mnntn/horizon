//! The `can_equip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canEquip`: may `stack` go into the equipment
    /// slot `slot` (`head`, `chest`, `legs`, `feet`, `mainhand`, `offhand`) of
    /// the entity `entity` of type `kind` (`minecraft:player`)? `can_equip`
    /// arrives holding the data answer: the stack's `equippable` component names
    /// that slot and lists the entity type among its `allowed_entities` (or lists
    /// none), as `LivingEntity.isEquippableInSlot` and
    /// `Equippable.canBeEquippedBy` read it. Set it either way. Asked where
    /// vanilla asks: the armour slots of the player's inventory menu
    /// (`ArmorSlot.mayPlace`, which a shift-click into one reaches too), a
    /// dispenser equipping what stands in front of it
    /// (`EquipmentDispenseItemBehavior`, one entity at a time, each asked for
    /// `entity`) and a mob picking an item up off the ground
    /// (`Mob.equipItemIfPossible`). It decides whether the stack fits the slot,
    /// not what the slot holds, whether the entity may wear one at all
    /// (`canUseSlot`) or the dispenser's own rules for the entity. Only the
    /// `ItemPart`s attached to `stack`'s item are asked, on both sides, as the
    /// client's menu predicts the slot: answer from the arguments and `context`
    /// alone, and fast. It has no `Host`, as `menu_slot_rules` has none.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        context = "loop",
        cancellable = false,
        target = "horizon_client_lib::mobs::equip::native_can_equip",
        neoforge = "IItemExtension.canEquip"
    )]
    fn can_equip();
}
