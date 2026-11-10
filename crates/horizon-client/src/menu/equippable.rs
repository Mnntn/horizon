//! `DataComponents.EQUIPPABLE` as a menu reads it from a stack: its slot and
//! the entity types that may wear it (`LivingEntity.isEquippableInSlot`).
use super::{Env, Menu};
use crate::items::ItemStack;
use horizon_core::nbt::Tag;
use std::collections::HashSet;

/// A stack's `Equippable`: its slot and `allowed_entities` (none = any).
pub(super) struct Equippable {
    pub slot: String,
    allowed: Option<HashSet<String>>,
}

impl Equippable {
    /// `Equippable.canBeEquippedBy`.
    pub fn allows(&self, entity: &str) -> bool {
        self.allowed.as_ref().is_none_or(|a| a.contains(entity))
    }
}

/// The equipment slots of `Role::Armor`, by `EquipmentSlot.getIndex` (feet first).
pub(super) const ARMOR_SLOTS: [&str; 4] = ["feet", "legs", "chest", "head"];

impl Menu {
    /// `ArmorSlot.mayPlace`: `LivingEntity.isEquippableInSlot` for the player of `env`, armour
    /// slot `eq` of [`ARMOR_SLOTS`]. The stack's `equippable` component answers first and the
    /// parts of its item may change that (`IItemExtension.canEquip`).
    pub(super) fn armor_may_place(env: &Env, eq: usize, stack: &ItemStack) -> bool {
        let data = Self::equippable(env, stack)
            .is_some_and(|e| ARMOR_SLOTS[eq] == e.slot && e.allows("minecraft:player"));
        env.player_entity_id.map_or(data, |player| {
            crate::mobs::equip::can_equip(
                env.loop_context,
                stack,
                ARMOR_SLOTS[eq],
                player as u32,
                "minecraft:player",
                data,
            )
        })
    }

    /// `ItemStack.get(DataComponents.EQUIPPABLE)`: the stack's component,
    /// else the item's default; a removed component leaves none.
    pub(super) fn equippable(env: &Env, stack: &ItemStack) -> Option<Equippable> {
        let data = stack.data();
        if data.is_some_and(|d| d.other("!minecraft:equippable").is_some()) {
            return None;
        }
        if let Some(tag) = data.and_then(|d| d.other("minecraft:equippable")) {
            let Some(Tag::String(slot)) = tag.get("slot") else {
                return None;
            };
            // A `HolderSet<EntityType>`: one id, a `#tag`, or a list of ids.
            let tags = &env.enchantments.entity_tags;
            let spec = |s: &Tag| match s {
                Tag::String(s) => {
                    let s = s.to_string_lossy();
                    match s.strip_prefix('#') {
                        Some(t) => tags.get(&qualified(t)).cloned().unwrap_or_default(),
                        None => HashSet::from([qualified(&s)]),
                    }
                }
                _ => HashSet::new(),
            };
            let allowed = match tag.get("allowed_entities") {
                Some(Tag::List(list)) => Some(list.iter().flat_map(spec).collect()),
                Some(one) => Some(spec(one)),
                None => None,
            };
            return Some(Equippable {
                slot: slot.to_string_lossy(),
                allowed,
            });
        }
        let info = env.items.get(&stack.item)?;
        Some(Equippable {
            slot: info.equip_slot.clone()?,
            allowed: info.equip_allowed.clone(),
        })
    }
}

/// `ResourceLocation.parse`: an id without a namespace is `minecraft:`.
fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::tests::{stack, with_menu};
    use super::super::{Input, Kind};
    use horizon_core::nbt::Tag;

    /// A stone whose `equippable` component has `fields`.
    pub(in crate::menu) fn equippable(fields: Vec<(&str, Tag)>) -> Option<crate::items::ItemStack> {
        let mut stone = stack("stone", 1)?;
        stone
            .data_mut()
            .set_other("minecraft:equippable", Some(Tag::compound(fields)));
        Some(stone)
    }

    /// A menu reads a stack's `EQUIPPABLE.slot`: a patched stone can go to
    /// the head slot and is equipped there.
    #[test]
    fn a_menu_reads_a_stacks_equippable_slot() {
        with_menu(Kind::Inventory, |env, menu| {
            let stone = equippable(vec![("slot", Tag::String("head".into()))]);
            menu.set_slot(env, 9, stone);
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(5).map(|s| &*s.item), Some("minecraft:stone"));
            assert_eq!(menu.slot(9), None);
            assert_eq!(menu.equipped.last().map(|(s, _)| *s), Some("head"));
        });
    }

    /// `allowed_entities` comes from the same stack component: a head item
    /// only a horse may wear stays out of the player's head slot.
    #[test]
    fn a_stacks_allowed_entities_keep_it_off_the_player() {
        with_menu(Kind::Inventory, |env, menu| {
            let stone = equippable(vec![
                ("slot", Tag::String("head".into())),
                ("allowed_entities", Tag::String("horse".into())),
            ]);
            menu.set_slot(env, 9, stone);
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(5), None);
        });
    }
}
