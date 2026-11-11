//! Recorded projectile weapons and the death-loot producer.
use super::*;

/// A weapon belonging to the victim's committed last damage source.
pub(in crate::game) struct AcceptedWeapon {
    source: (Damage, i64),
    stack: Option<ItemStack>,
}
impl AcceptedWeapon {
    fn belongs_to(&self, damage: Option<(Damage, i64)>) -> bool {
        damage == Some(self.source)
    }
}

impl Game<'_> {
    pub(in crate::game) fn capture_projectile_weapon(
        &mut self,
        id: u32,
        source: (Damage, i64),
        stack: Option<&ItemStack>,
    ) {
        if mod_api::__private::listened(mod_api::HookId::LootDrops) {
            self.combat.death_weapons.insert(
                id,
                AcceptedWeapon {
                    source,
                    stack: stack.cloned(),
                },
            );
        }
    }

    /// Commit the weapon only after hurtServer committed this projectile's source.
    pub(in crate::game) fn damage_projectile_with_weapon(
        &mut self,
        projectile: &Projectile,
        id: u32,
        amount: f32,
        source: Damage,
        aim: Option<horizon_client::mobs::Aim>,
    ) -> bool {
        let (hurt, accepted) =
            self.mobs
                .iter_mut()
                .find(|mob| mob.id == id)
                .map_or((false, None), |mob| {
                    let hurt = mob.damage_projectile(
                        crate::game::entity::damage_level_of!(self, scoped),
                        amount,
                        source,
                        aim,
                        [projectile.velocity.x, projectile.velocity.z],
                    );
                    // An immune or wholly blocked hit returns true without committing a source.
                    let changed = mod_api::__private::listened(mod_api::HookId::LootDrops)
                        && mob.projectile_damage_committed;
                    (hurt, changed.then_some(mob.last_damage).flatten())
                });
        if let Some(accepted) = accepted {
            let stack = projectile.arrow.as_ref().and_then(|arrow| {
                if matches!(projectile.kind, ProjectileKind::Trident) {
                    Some(&arrow.pickup_item)
                } else {
                    arrow.fired_from_weapon.as_ref()
                }
            });
            self.capture_projectile_weapon(id, accepted, stack);
        }
        hurt
    }
    /// What an arrow or a thrown trident hits an entity of type `victim`
    /// for (`AbstractArrow.onHitEntity`, `ThrownTrident.onHitEntity`): a
    /// trident's 8 through `EnchantmentHelper.modifyDamage` of the trident
    /// (Impaling's `minecraft:damage` on `#sensitive_to_impaling`).
    pub(in crate::game) fn shot_damage(
        &mut self,
        shot: &mut Projectile,
        victim: Option<&str>,
        who: Who,
    ) -> f32 {
        let trident = matches!(shot.kind, ProjectileKind::Trident);
        let base = match shot.kind {
            ProjectileKind::Arrow { base_damage, .. } => base_damage as f32,
            _ => 8.0,
        };
        let weapon = shot.arrow.as_ref().and_then(|a| {
            if trident {
                Some(&a.pickup_item)
            } else {
                a.fired_from_weapon.as_ref()
            }
        });
        let Some(weapon) = weapon else {
            return if trident {
                base
            } else {
                shot.arrow_damage() + shot.bonus.damage
            };
        };
        let attacker_kind = match shot.owner {
            Some(Who::Player) => Some("minecraft:player".to_owned()),
            Some(Who::Mob(id)) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.kind.id.clone()),
            None => None,
        };
        let ctx = horizon_client::enchant::EffectContext {
            this: victim,
            attacker: attacker_kind.as_deref(),
            direct_attacker: Some(if trident {
                "minecraft:trident"
            } else if shot.is_spectral_arrow() {
                "minecraft:spectral_arrow"
            } else {
                "minecraft:arrow"
            }),
            damage_type: Some(if trident {
                "minecraft:trident"
            } else {
                "minecraft:arrow"
            }),
            ..Default::default()
        };
        let player = self.own_entity_id() as u32;
        let attacker = shot.owner.map(|owner| match owner {
            Who::Player => self
                .combat
                .enchantment_projectile_attacker
                .or_else(|| self.player_attacker(shot.guest).map(|s| s.0))
                .unwrap_or(player),
            Who::Mob(id) => id,
        });
        let victim = match who {
            Who::Player => player,
            Who::Mob(id) => id,
        };
        let victim =
            if (crate::game::dragon::SPECIAL..crate::game::dragon::CRYSTAL_IDS).contains(&victim) {
                self.dragon.as_ref().map_or(victim, |dragon| dragon.id)
            } else if victim >= crate::game::dragon::CRYSTAL_IDS {
                self.crystals
                    .get((victim - crate::game::dragon::CRYSTAL_IDS) as usize)
                    .map_or(victim, |crystal| crystal.id)
            } else {
                victim
            };
        let enchanted =
            self.evaluate_weapon_damage(weapon, attacker.map(|id| (id, victim)), ctx, base);
        if trident {
            enchanted
        } else {
            shot.arrow_damage_with_base(f64::from(enchanted)) + shot.bonus.damage
        }
    }
    pub(super) fn drop_death_loot(
        &mut self,
        m: &mut Mob,
        player_uuid: Option<u128>,
        player_killed: bool,
        killer: horizon_world::loot::LootEntity,
        consumed: bool,
        pos: [f64; 3],
        source: Option<(&'static str, u32)>,
    ) {
        use horizon_client::mobs::{DamageKind, Who, drops};
        use horizon_world::loot::{LootDamage, LootEntity};
        let weapon = self
            .combat
            .death_weapons
            .remove(&m.id)
            .filter(|weapon| weapon.belongs_to(m.last_damage))
            .and_then(|weapon| weapon.stack);
        let weapon = weapon.and_then(|stack| stack.try_into_api().ok());
        let Some(loot) = self.server_loot.get() else {
            return;
        };
        let player = || killer.clone();
        let damage = m.last_damage.map(|(d, _)| {
            let source = match d.attacker {
                Some(Who::Player) => Some(player()),
                Some(Who::Mob(id)) => self
                    .mobs
                    .iter()
                    .find(|o| o.id == id)
                    .map(|o| drops::loot_entity(o, None))
                    // A charged creeper's blast kills after the creeper left the list.
                    .or_else(|| {
                        self.combat
                            .charged_exploders
                            .contains(&id)
                            .then(|| LootEntity {
                                powered: true,
                                ..LootEntity::new("minecraft:creeper")
                            })
                    }),
                None => None,
            };
            let direct = match d.kind {
                DamageKind::Arrow => Some(LootEntity {
                    kind: "minecraft:arrow".into(),
                    ..LootEntity::default()
                }),
                DamageKind::Fireball => Some(LootEntity {
                    kind: "minecraft:fireball".into(),
                    ..LootEntity::default()
                }),
                DamageKind::WitherSkull => Some(LootEntity {
                    kind: "minecraft:wither_skull".into(),
                    ..LootEntity::default()
                }),
                DamageKind::Fireworks => Some(LootEntity {
                    kind: "minecraft:firework_rocket".into(),
                    ..LootEntity::default()
                }),
                _ => source.clone(),
            };
            LootDamage {
                kind: source_damage_type(d).into(),
                source,
                direct,
            }
        });
        let vehicle = m
            .extra
            .mount
            .and_then(|id| self.mobs.iter().find(|o| o.id == id))
            .map(|o| LootEntity::new(&o.kind.id));
        let kind = crate::game::native_hooks::mob_kind(m);
        let death = drops::Death {
            loot: &loot,
            damage,
            player: player_uuid.is_some().then(player),
            player_killed,
            luck: 0.0,
            vehicle,
            mob_drops: self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::MOB_DROPS),
            world_seed: self.level.seed,
        };
        let raider = horizon_client::mobs::raider::is_raider(m.species());
        let to_stack = |stack: &horizon_world::loot::LootStack| {
            if raider && stack.item == horizon_client::mobs::raider::BANNER {
                horizon_client::mobs::raider::ominous_banner()
            } else {
                from_loot(stack)
            }
        };
        // LootContextParams.ORIGIN of a death: the mob's position.
        let place = self.loot_place_at([m.position.x, m.position.y, m.position.z], None);
        let hurt = source.map(|(damage_type, causing)| {
            let mut source = death_hurt_source(damage_type, causing);
            if m.last_direct.is_some() {
                source.direct = 0;
            }
            source
        });
        let enchanted_loot = hurt
            .as_ref()
            .filter(|_| mod_api::__private::listened(mod_api::HookId::EnchantedEntityLoot))
            .zip(self.effect_living_ref(Some(m.id as i32)));
        let (items, skulls) = horizon_world::loot::with_native_loot_place(place, || {
            let rolled = match enchanted_loot {
                Some((source, entity)) => {
                    let mut rolls =
                        crate::game::loot_rolls::ListenedRolls::new(self, entity, source);
                    drops::roll_death(m, &death, &mut rolls)
                }
                None => {
                    let mut rolls = drops::PlainRolls {
                        sequences: &mut self.rand_sequences,
                        level_random: &mut self.world.random,
                    };
                    drops::roll_death(m, &death, &mut rolls)
                }
            };
            let Some(hurt) = &hurt else {
                return drops::finish_death(m, &death, rolled, None);
            };
            // GetEquipmentDropChanceEvent, asked for each slot just before its roll.
            let id = m.id;
            let mut choose = |choice: drops::EquipmentChoice<'_>| {
                let stack = match choice.stack {
                    drops::Drop::Item(stack) => to_stack(stack),
                    drops::Drop::Stack(stack) => stack.clone(),
                    _ => return choice.chance,
                };
                let mut chance = choice.chance;
                let Ok(stack) = stack.try_into_api() else {
                    return chance;
                };
                self.native_get_equipment_drop_chance(
                    mod_api::__private::GetEquipmentDropChanceCall {
                        entity: id,
                        kind: &kind,
                        slot: EquipmentSlot::ALL[choice.slot],
                        source: hurt,
                        recently_hit: player_killed,
                        stack: &stack,
                        original_chance: choice.original,
                        preserved: choice.preserved,
                        chance: &mut chance,
                    },
                );
                chance
            };
            drops::finish_death(m, &death, rolled, Some(&mut choose))
        });
        if skulls && let Some(Who::Mob(id)) = m.last_damage.and_then(|(d, _)| d.attacker) {
            // Creeper.droppedSkulls: one head per charged explosion.
            self.combat.charged_exploders.retain(|c| *c != id);
            if let Some(creeper) = self.mobs.iter_mut().find(|o| o.id == id) {
                creeper.extra.charged = false;
            }
        }
        // The item drops come before the carried block and the orbs, so the
        // mods get them all at once and they still spawn in that order.
        let mut stacks = Vec::new();
        let mut rest = Vec::new();
        for d in items {
            match d {
                drops::Drop::Item(stack) => stacks.push(to_stack(&stack)),
                drops::Drop::Stack(stack) => stacks.push(stack),
                other => rest.push(other),
            }
        }
        let table = drops::death_loot_table(m, death.mob_drops).filter(|t| loot.has_table(t));
        let context = LootDrops::from(mod_api::__private::built::LootDrops {
            damage_source: hurt,
            weapon: weapon.as_ref(),
            table: table.as_deref().unwrap_or_default(),
            dimension: self.dim.id(),
            origin: [m.position.x, m.position.y, m.position.z],
            block: None,
            entity: Some((m.id, &kind)),
            player: player_uuid,
            tool: None,
            explosion: None,
            used: false,
        });
        self.native_loot_drops(&context, &mut stacks);
        stacks.retain(|stack| stack.count > 0 && &*stack.item != "minecraft:air");
        let mut experience = None;
        for d in rest {
            match d {
                drops::Drop::CarriedBlock(state) => {
                    // Enderman.dropCustomDeathLoot: spawnAtLocation for each
                    // of the carried block's silk-touch drops.
                    use horizon_client::mobs::enderman;
                    let tool = enderman::carried_block_tool();
                    let at = [m.position.x, m.position.y, m.position.z];
                    let drops = self.roll_block_loot(
                        at.map(|c| c.floor() as i32),
                        state,
                        Some(m.id),
                        enderman::carried_block_roll(&tool),
                    );
                    stacks.extend(drops.into_iter().filter(enderman::carried_block_drop_kept));
                }
                drops::Drop::Experience(reward) if !consumed => experience = Some(reward),
                drops::Drop::Item(_) | drops::Drop::Stack(_) | drops::Drop::Experience(_) => {}
            }
        }
        let mut stacks: Vec<(ItemStack, [f64; 3])> =
            stacks.into_iter().map(|stack| (stack, pos)).collect();
        if m.species() == Species::CopperGolem {
            // CopperGolem.dropEquipment: the preserved equipment is part of
            // dropAllDeathLoot, so the mods hear it with the rest. It is
            // `spawnAtLocation` with no offset: at the golem's feet.
            let feet = [m.position.x, m.position.y, m.position.z];
            stacks.extend(
                Self::golem_preserved_equipment(m)
                    .into_iter()
                    .map(|stack| (stack, feet)),
            );
        }
        // Every death drop is its own ItemEntity, and the drop entity's own
        // random (not the level's) gives it the constructor's horizontal push
        // (its motion otherwise stays [0, 0.2, 0] and the drops land stacked
        // at the mob's centre, without disturbing the level random that the
        // orbs' merge ids and block ticks read next).
        let dying = DyingDrops {
            entity: m.id,
            kind: &kind,
            player: None,
            source: hurt,
            recently_hit: player_killed,
        };
        self.drop_death_items(&dying, stacks, 10, |_| None);
        if let Some(reward) = experience {
            let amount = self.living_experience_drop(m.id, &kind, player_uuid, reward);
            self.spawn_orbs(pos, amount as u32);
        }
    }
}

#[cfg(test)]
#[path = "death_weapon_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "death_weapon_review_tests.rs"]
mod review_tests;
