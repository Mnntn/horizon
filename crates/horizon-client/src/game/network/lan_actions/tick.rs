use super::*;

impl Game<'_> {
    /// Per tick, for every guest: dig, use items, pick up drops, die.
    pub(in crate::game) fn tick_guests(&mut self) {
        building_cache(self.lan.as_ref());
        self.tick_guests_inner();
        building_cache(None);
        self.pickup_columns = None;
    }

    fn tick_guests_inner(&mut self) {
        // `EntityEvent.Size` for the poses the guests' moves set since the last tick.
        self.native_guest_sizes();
        self.tick_guest_cameras();
        self.lan_merchant_tick();
        self.lan_stations_tick();
        self.lan_tables_tick();
        self.lan_mod_crafting_tick();
        self.lan_menu_links_tick();
        self.lan_containers_tick();
        self.tick_lan_mount_menus();
        self.tick_player_menus();
        self.native_players_equipment();
        self.announce_guest_advancements();
        // Entity.commonTick belongs to each player, including spectators and
        // players whose client has not loaded yet, not to the server clock.
        if let Some(host) = self.lan.as_mut() {
            for g in &host.hub.guests {
                if g.participates_in_player_simulation()
                    && host.hub.here(g)
                    && !self.server_trips.iter().any(|t| t.is(g.conn))
                {
                    host.play.entry(g.conn).or_default().level_up_sound.tick();
                }
            }
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let conns: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| host.hub.here(g) && g.loaded)
            .map(|g| g.conn)
            .collect();
        // The players' pickups ask which columns items reach before
        // scanning every item: most players stand on none.
        self.pickup_columns = Some(Default::default());
        // The players the guests' hooks can catch.
        if host.play.values().any(|p| p.hook.is_some()) {
            self.refresh_hook_targets();
        }
        for conn in conns {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
                continue;
            };
            // ServerGamePacketListenerImpl.tick: the chat and command throttlers
            // decay one a tick, dead or alive.
            let play = host.play.entry(conn).or_default();
            play.chat_spam = play.chat_spam.saturating_sub(1);
            play.command_spam = play.command_spam.saturating_sub(1);
            play.recipe_checks = play.recipe_checks.saturating_sub(1);
            // commandSuggestionsProvider.tick: one answer a tick, to the latest request.
            play.suggestion_budget = true;
            self.guest_try_suggestions(conn);
            self.guest_player_tick(gi, conn, mod_api::HookId::PlayerTickPre, |game, player| {
                game.native_player_tick_pre(mod_api::__private::PlayerTickPreCall { player })
            });
            self.guest_sleep_tick(gi);
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.guests[gi].dead {
                self.guest_tick_death(gi);
                if let Some(host) = self.lan.as_ref() {
                    refresh_building_player(host, conn);
                }
                self.guest_player_tick(
                    gi,
                    conn,
                    mod_api::HookId::PlayerTickPost,
                    |game, player| {
                        game.native_player_tick_post(mod_api::__private::PlayerTickPostCall {
                            player,
                        })
                    },
                );
                continue;
            }
            if self.on_server_trip(conn) {
                // LivingEntity.hurt kills at once (die), whether the level
                // has loaded around the player or not: its respawn request
                // is taken as soon as it comes.
                self.guest_die(gi, conn);
                if let Some(host) = self.lan.as_ref() {
                    refresh_building_player(host, conn);
                }
                self.guest_player_tick(
                    gi,
                    conn,
                    mod_api::HookId::PlayerTickPost,
                    |game, player| {
                        game.native_player_tick_post(mod_api::__private::PlayerTickPostCall {
                            player,
                        })
                    },
                );
                continue;
            }
            // ServerPlayerGameMode.tick stops an active destroy when its block is air.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if let Some((pos, _)) = host.play.get(&conn).and_then(|play| play.digging)
                && self
                    .res
                    .facts()
                    .is_air(self.world.block(pos[0], pos[1], pos[2]))
                && let Some(play) = host.play.get_mut(&conn)
            {
                play.digging = None;
                play.mining = None;
            }
            let play = host.play.entry(conn).or_default();
            // ServerGamePacketListenerImpl.tick's dropSpamThrottler.
            play.creative_drops = play.creative_drops.saturating_sub(1);
            let digging = play.digging;
            let using = play.using.is_some();
            let delayed = play.destroy_delay > 0 || play.use_delay > 0 || play.attack_ticks < 100;
            if digging.is_some() || using || delayed {
                let players = if using {
                    self.stab_players_for(conn)
                } else {
                    Vec::new()
                };
                self.as_guest(conn, |g| {
                    let outer = std::mem::replace(&mut g.stab_players, players);
                    g.attack_ticks = g.attack_ticks.saturating_add(1);
                    if let Some((pos, face)) = digging {
                        g.forced_hit = Some(Hit {
                            pos,
                            face,
                            point: [
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]) + 0.5,
                                f64::from(pos[2]) + 0.5,
                            ],
                            inside: false,
                        });
                    }
                    let _ = g.tick_mining();
                    g.forced_hit = None;
                    g.right = g.using.is_some();
                    g.tick_using();
                    g.stab_players = outer;
                    if g.use_delay > 0 {
                        g.use_delay -= 1;
                    }
                });
                self.land_player_stabs();
                // ServerPlayerGameMode.handleBlockBreakAction: creative and
                // instant survival STARTs finish without a STOP from the client.
                // tick_mining clears mining on completion; forget the START too,
                // so a flower or crop planted here later is not broken again.
                if digging.is_some()
                    && let Some(host) = self.lan.as_mut()
                    && let Some(play) = host.play.get_mut(&conn)
                    && (host.hub.guests[gi].creative() || play.mining.is_none())
                {
                    play.digging = None;
                }
            }
            // Player.tick's ItemCooldowns.tick, once a tick for each player
            // (the shield an axe disabled, a thrown pearl), after
            // LivingEntity.tick's use: an entry a finished use starts has 20
            // ticks left when the next tick's packets are handled, as the
            // guest's own client counts.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let play = host.play.entry(conn).or_default();
            play.shield_cooldown = play.shield_cooldown.saturating_sub(1);
            // ServerItemCooldowns.onCooldownEnded: a finished entry is told to
            // the guest (a duration of 0 removes it).
            for group in play.cooldowns.tick() {
                host.hub.send_to(
                    conn,
                    &C::Cooldown(play::Cooldown {
                        group: group.to_string(),
                        duration: 0,
                    }),
                );
            }
            // Player.tick: a different main-hand item resets the attack
            // strength, and detectEquipmentUpdates gives the guest its modifiers.
            let g = &host.hub.guests[gi];
            let held = g
                .inventory
                .get(g.selected)
                .cloned()
                .flatten()
                .map(|s| s.item);
            let play = host.play.entry(conn).or_default();
            if play.attack_item != held {
                play.attack_ticks = 0;
                play.attack_item = held;
            }
            self.guest_delayed_destroy(conn);
            self.guest_server_tick(conn);
            // Player.tick's statistics and ServerPlayer.tick's triggers, then
            // PlayerAdvancements.flushDirty.
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let sleeping = host.hub.guests[gi].sleeping;
            // ServerPlayer.doTick's updateScoreForCriteria: the health, food,
            // armor, experience and level objectives follow the guest.
            let mut recorded =
                std::mem::take(&mut host.play.entry(conn).or_default().score_recorded);
            self.as_guest(conn, |g| {
                g.tick_fishing();
                g.tick_maps();
                g.tick_lodestone_compasses();
                g.native_inventory_ticks();
                g.count_tick_stats(sleeping);
                g.tick_advancements();
                g.update_player_attributes();
                g.record_player_scores(&mut recorded);
            });
            if let Some(host) = self.lan.as_mut() {
                host.play.entry(conn).or_default().score_recorded = recorded;
            }
            self.lan_map_updates(conn);
            if let Some(host) = self.lan.as_mut() {
                host.hub.flush_progress(conn);
            }
            self.guest_dig_feedback(conn);
            // A hand-over freezes this player's inventory until its record is released.
            if self
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.guests[gi].handover.is_none())
            {
                self.guest_pickups(gi);
                self.guest_arrows(gi);
                self.guest_orbs(gi);
            }
            self.guest_die(gi, conn);
            if let Some(host) = self.lan.as_ref() {
                refresh_building_player(host, conn);
            }
            self.guest_player_tick(gi, conn, mod_api::HookId::PlayerTickPost, |game, player| {
                game.native_player_tick_post(mod_api::__private::PlayerTickPostCall { player })
            });
        }
    }

    /// Only listeners pay for a guest swap; reuse the loop's resolved index.
    pub(super) fn guest_player_tick(
        &mut self,
        gi: usize,
        conn: u32,
        hook: mod_api::HookId,
        fire: fn(&mut Self, mod_api::PlayerRef),
    ) {
        if !mod_api::__private::listened(hook) {
            return;
        }
        let (Some(epoch), Some(host)) = (self.native_lifecycle.epoch(), self.lan.as_ref()) else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, guest.entity_id as u32),
            guest.uuid,
        );
        let _ = self.as_guest_at(gi, conn, |game| fire(game, player));
    }

    /// `LivingEntity.checkAutoSpinAttack` for a server's copy of a Riptide
    /// spinner (it runs before `aiStep`'s countdown clears the damage): the
    /// first mob touching the player's box takes `Player.attack` with the
    /// spin's damage and the spin ends. A `ServerPlayer` is client
    /// authoritative, so its `aiStep` does not travel and both boxes vanilla
    /// passes are the current one: no sweep over the tick's moves.
    pub(in crate::game) fn server_spin_attack(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if p.auto_spin_attack_ticks <= 0 {
            return;
        }
        let swept = p.bounds();
        let riding = self.riding;
        let Some(index) = first_spin_target(
            swept,
            riding,
            self.mobs.iter().map(|m| (m.id, m.removed, m.bounds())),
        ) else {
            return;
        };
        self.attack_mob(index);
        if let Some(p) = self.player.as_mut() {
            p.auto_spin_attack_ticks = 0;
            p.auto_spin_attack_dmg = 0.0;
            p.velocity = p.velocity.scale(-0.2);
        }
    }

    /// `handlePlayerPositionChange` jumps from the pre-move position and yaw.
    /// A later attack packet reads the resulting velocity even before travel.
    pub(in crate::game) fn guest_jump(&mut self, conn: u32, from: [f64; 3], yaw: f32) {
        let identity = if !mod_api::__private::listened(mod_api::HookId::PlayerJump) {
            None
        } else {
            self.lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .map(|g| (g.uuid, g.name.clone()))
        };
        self.as_guest(conn, |game| {
            let Some(player) = game.player.as_mut() else {
                return;
            };
            let (position, yaw_after, ground) =
                (player.position, player.yaw, player.ground.on_ground);
            player.position = Vec3::new(from[0], from[1], from[2]);
            player.yaw = yaw;
            player.ground.on_ground = true;
            let mut w = MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
                health: &game.health_resources,
                tag_items: Some(world_items!(game)),
                loop_context: &game.loop_context,
                damage_hooks: &game.damage_hooks,
                block_registries: Some(&game.pack.block_registries),
            };
            let _ = w.jump_from_ground();
            (w.player.position, w.player.yaw, w.player.ground.on_ground) =
                (position, yaw_after, ground);
            if let Some((uuid, name)) = identity {
                game.native_player_jump(uuid, &name);
            }
        });
    }

    /// The guest's body on the server each tick: ServerPlayer.tick's damage
    /// clock and Entity.baseTick's fluid phase (burning), then for each move
    /// ServerPlayer.doCheckFallDamage (the fall distance grows while they go
    /// down and hurts them where they land, as the host's own landing does;
    /// water resets it, lava halves it) and checkMovementStatistics (the
    /// exhaustion of sprinting and swimming), then LivingEntity.baseTick's
    /// hazards (drowning, lava, fire blocks, suffocation, the void), and
    /// regeneration and the food tick (ServerPlayer.doTick). Their hurt
    /// sounds are heard at the guest.
    pub(super) fn guest_server_tick(&mut self, conn: u32) {
        self.guest_replay(conn);
        self.guest_vehicle_tick(conn);
        // The last move's fall stands in for the server's delta movement.
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let fell = host
            .hub
            .guest(conn)
            .and_then(|g| g.moves.last().map(|m| m.0[1]))
            .unwrap_or(0.0);
        self.guest_glide_tick(conn, fell);
        let Some((moves, from_ground)) = self.lan.as_mut().and_then(|host| {
            host.hub.update_guest(conn, |g| {
                let moves = std::mem::take(&mut g.moves);
                let from_ground = if moves.is_empty() {
                    g.on_ground
                } else {
                    g.moves_from_ground
                };
                (moves, from_ground)
            })
        }) else {
            return;
        };
        let g = self.lan.as_ref().unwrap().hub.guest(conn).unwrap();
        if g.spectator() {
            // ServerPlayer.doTick runs the player's whole tick for a spectator
            // too, unless its box touches an unloaded chunk
            // (`touchingUnloadedChunk`): its effects (LivingEntity.baseTick),
            // Player.aiStep's tickRegeneration and foodData.tick go on while
            // it watches (#11312). The hazards, the travel and the movement
            // statistics below are what a spectator's body skips.
            let (at, body) = (g.position, g.dimensions());
            let loaded =
                !touching_unloaded_chunk(at, body, |x, z| self.world.chunks.contains_key(&(x, z)));
            if !g.effects.list.is_empty() {
                self.as_guest(conn, |game| game.tick_mob_effects());
            }
            self.as_guest(conn, |game| {
                game.handle_shoulder_entities();
                if !loaded {
                    return;
                }
                let rules = game.world.health_rules;
                let Some(player) = game.player.as_mut() else {
                    return;
                };
                // Entity.baseTick's clocks (hurtTime, invulnerableTime) run too.
                player.health.advance_clock();
                let _ = player.peaceful_regeneration(&game.loop_context, rules);
                let _ = player.food_tick(
                    &game.loop_context,
                    crate::game::entity::damage_level_of!(game),
                    &game.health_resources,
                    rules,
                );
                let events = std::mem::take(&mut player.events);
                game.record_combat(&events);
            });
            return;
        }
        let at = g.position;
        let dimensions = g.dimensions();
        let (sneaking, sleeping) = (g.sneaking, g.sleeping);
        let riding = g.vehicle.is_some();
        if g.motion
            .as_ref()
            .is_some_and(|m| m.auto_spin_attack_ticks > 0)
        {
            self.as_guest(conn, |game| game.server_spin_attack());
        }
        let in_wall = !sleeping && self.in_wall(at, dimensions);
        let mut soul = None;
        let events = self.as_guest(conn, |game| {
            let (rules, motion) = (game.world.health_rules, game.motion);
            let health = &motion.health;
            let freeze_immune = game.wears_freeze_immune();
            let burn_immune = game.wears_burn_from_stepping_immunity();
            let boat = game.riding_boat_fluid_box();
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None);
            };
            player.fluids.boat = boat;
            let mut w = MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
                health: &game.health_resources,
                tag_items: Some(world_items!(game)),
                loop_context: &game.loop_context,
                damage_hooks: &game.damage_hooks,
                block_registries: Some(&game.pack.block_registries),
            };
            w.player.health.advance_clock();
            let _ = w.fluid_base_tick();
            w.player.ground.on_ground = from_ground;
            // Player.aiStep: a flying player that isn't riding starts every tick
            // with no fall distance, so flying down in creative and switching to
            // survival lands without the flight's distance.
            if w.player.flying && !riding {
                w.player.fall_distance = 0.0;
            }
            let travelled: Vec<[f64; 3]> = moves.iter().map(|m| m.0).collect();
            // The tick's whole movement, for the delta movement a glide's wall
            // hit reads (`handleMovePlayer` gives the server copy the client's
            // own movement).
            let tick_movement = travelled.iter().fold(Vec3::ZERO, |sum, d| {
                sum.add_vector(Vec3::new(d[0], d[1], d[2]))
            });
            // Entity.move's movementThisTick: each move, from where the
            // tick's first one started.
            let end = w.player.position;
            let start = travelled
                .iter()
                .fold(end, |p, d| p.subtract(Vec3::new(d[0], d[1], d[2])));
            w.player.history.clear();
            let mut from = start;
            for d in &travelled {
                let delta = Vec3::new(d[0], d[1], d[2]);
                let to = from.add_vector(delta);
                w.player
                    .history
                    .add(horizon_world::entity_move::MovementRecord {
                        from,
                        to,
                        original: Some(delta),
                    });
                from = to;
            }
            let _ = w.server_stuck_in_blocks(start);
            for (moved, on_ground) in moves {
                let movement = Vec3::new(moved[0], moved[1], moved[2]);
                let landed = (|| -> horizon_client::Result<()> {
                    w.set_on_ground_with_movement(on_ground, false, movement)?;
                    let pos = w.legacy_on_position()?;
                    let state = w.block_state(pos)?;
                    w.check_fall_damage(moved[1], on_ground, state, pos)?;
                    w.check_movement_statistics(movement)
                })();
                if landed.is_err() {
                    break;
                }
            }
            // A ServerPlayer travels on the server too
            // (MoveSimulationType.AUTHORITATIVE_SIDE_AND_SERVER), so every tick
            // on a ladder or vine LivingEntity.handleOnClimbable resets its fall
            // distance: climbing down doesn't hurt where it lands (#1255).
            if w.climbable().unwrap_or(false) {
                w.player.fall_distance = 0.0;
            }
            // ServerGamePacketListenerImpl.handlePlayerPositionChange ends the
            // impulse context on LivingEntity.hasLandedInLiquid: a move into a
            // liquid that does not rise, or a wind charge that lands in water
            // would waive the fall after it.
            let liquid = w.player.fluids.was_touching_water || w.player.is_in_lava();
            let delta_y = w.player.velocity.y;
            w.player.impulse.landed_in_liquid(delta_y, liquid);
            // Entity.applyEffectsFromBlocks along those moves: magma, cactus,
            // lit campfires and grown sweet berry bushes hurt (#1305).
            w.player.shift_key_down = sneaking;
            let _ = w.player.set_sleeping(sleeping);
            w.player.freeze_immune = freeze_immune;
            w.player.burn_from_stepping_immune = burn_immune;
            let _ = w.server_block_damage(start);
            let _ = w.update_soul_speed_block();
            let _ = w.tick_environment_damage();
            let _ = w.apply_fire_contact();
            let _ = w.server_hazards_tick(in_wall);
            // LivingEntity.aiStep: a Riptide spin counts down on the server's
            // copy too, or it would stay in the spin pose (eye 0.4) for good.
            // LivingEntity.tick: a changed scale rebuilds the box. Player.tick
            // ends with updatePlayerPose — or, for a passenger, the rideTick
            // path that clears a swim (#4509 / #3308).
            let _ = if riding {
                w.passenger_pose_tick()
            } else {
                w.server_player_pose_tick()
            };
            let mut events = std::mem::take(&mut w.player.events);
            // LayeredCauldronBlock.handleEntityOnFireInside along those moves.
            for event in &events {
                match event {
                    MotionEvent::CauldronExtinguish { pos } => game.cauldron_extinguish(*pos),
                    MotionEvent::PowderSnowMelt { pos } => game.powder_snow_melt(*pos),
                    _ => {}
                }
            }
            // Entity.baseTick: rain puts out a burning player.
            game.rain_extinguish_player();
            // WitherRoseBlock.entityInside along those moves.
            for event in &events {
                if matches!(event, MotionEvent::InsideWitherRose) {
                    game.wither_rose_contact();
                }
            }
            // ServerGamePacketListenerImpl.handleMovePlayer:
            // Player.checkMovementStatistics for each move.
            for d in travelled {
                game.movement_stats(Vec3::new(d[0], d[1], d[2]));
            }
            // LivingEntity.baseTick: a new block position runs the worn
            // enchantments' location_changed effects (frost walker).
            game.location_changed_effects([start.x, start.y, start.z], from_ground);
            // LivingEntity.baseTick: outside the world border, then tickEffects.
            game.border_damage();
            game.tick_mob_effects();
            // Soul Speed's effects (LivingEntity.baseTick's location_changed
            // and tick effects); its particle goes to the players around.
            soul = game.tick_soul_speed();
            game.handle_shoulder_entities();
            let Some(player) = game.player.as_mut() else {
                return (Vec::new(), None);
            };
            let _ = player.peaceful_regeneration(&game.loop_context, rules);
            let _ = player.food_tick(
                &game.loop_context,
                crate::game::entity::damage_level_of!(game),
                health,
                rules,
            );
            events.append(&mut player.events);
            player.carry_client_movement(tick_movement);
            let _ = MotionWorld {
                entities: &[],
                player,
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
                health: &game.health_resources,
                tag_items: Some(world_items!(game)),
                loop_context: &game.loop_context,
                damage_hooks: &game.damage_hooks,
                block_registries: Some(&game.pack.block_registries),
            }
            .server_travel();
            // Player.aiStep's entity contacts run with the player's body,
            // even when the level and its mobs are frozen. Flush their damage
            // events and marked velocity through the existing guest path.
            game.touch_pufferfish();
            let Some(player) = game.player.as_mut() else {
                return (events, None);
            };
            // Landing damage from this travel reaches the tracker too.
            events.append(&mut player.events);
            // A hurt since the last tick marked the velocity (Entity.markHurt):
            // ServerEntity.sendChanges sends it as it is after this travel.
            let marked = std::mem::take(&mut player.health.sync_velocity);
            let velocity = marked.then_some(player.velocity);
            // Player.causeFallDamage: a fall of 2 blocks or more counts.
            for event in &events {
                if let MotionEvent::Fall(horizon_client::landing::FallEvent::Statistic(cm)) = event
                {
                    game.award_custom_stat("fall_one_cm", *cm);
                }
            }
            game.record_combat(&events);
            game.wear_armor(&events);
            (events, velocity)
        });
        let Some((events, marked)) = events else {
            return;
        };
        if let Some((at, velocity)) = soul
            && let Some(host) = self.lan.as_ref()
        {
            crate::game::lan_particles::send_soul_particle(host, at, velocity);
        }
        self.as_guest(conn, |game| {
            game.sculk_step_on_player();
            game.tick_player_sculk_events();
            // Entity.restituteMovementAfterCollisions: GameEvent.BOUNCE.
            for event in &events {
                if matches!(event, MotionEvent::Bounce { .. }) {
                    game.post_entity_event("minecraft:bounce", horizon_client::mobs::Who::Player);
                }
            }
        });
        for event in events {
            match event {
                MotionEvent::Health(horizon_client::health::HealthEvent::DamageBroadcast {
                    source,
                }) => self.guest_damage_broadcast(conn, source),
                // LivingEntity.playHurtSound: the server's `Entity.playSound`
                // reaches every player around but the hurt one, whose own
                // client plays it from handleDamageEvent (#4149).
                MotionEvent::HurtSound(sound) => self.guest_hurt_sound(conn, sound),
                // LivingEntity.checkFallDamage: the crumbs reach every player.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Particles {
                    state,
                    position,
                    count,
                    speed,
                }) => self.landing_dust(state, position, count, speed),
                // A block's own sound names no player: the guest hears it too.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound {
                    sound,
                    position: Some(pos),
                    volume,
                    pitch,
                }) if sound.block_sound().is_some() => {
                    if let Some(name) = sound.block_sound() {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        self.audio.play_unshared(name, Some(at), volume, pitch);
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), None);
                    }
                }
                // LivingEntity.causeFallDamage, and HoneyBlock.fallOn's
                // `entity.playSound`: Player.playSound skips the guest, whose
                // own client plays it; everyone else hears it.
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound {
                    sound,
                    volume,
                    pitch,
                    ..
                }) => {
                    if let Some(name) = sound.player_fall_sound() {
                        self.audio.play_unshared(name, Some(at), volume, pitch);
                        self.send_level_sound(LevelSound::new(name, at, volume, pitch), Some(conn));
                    }
                }
                // Player.causeFallDamage, mayfly: the mods hear the landing.
                MotionEvent::Fall(horizon_client::landing::FallEvent::FlyableFall {
                    distance,
                    multiplier,
                }) => {
                    if let Some((uuid, id, name)) = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.uuid, g.entity_id, g.name.clone()))
                    {
                        self.as_guest(conn, |game| {
                            game.native_player_flyable_fall(uuid, id, &name, distance, multiplier);
                        });
                    }
                }
                // HoneyBlock.fallOn: entity event 54, including the player who
                // landed (`broadcastEntityEvent` has no excluded player).
                MotionEvent::Fall(horizon_client::landing::FallEvent::Broadcast(event))
                    if horizon_client::landing::honey_particle_count(event as i8).is_some() =>
                {
                    if let Some(host) = self.lan.as_ref()
                        && let Some(id) = host.hub.guest(conn).map(|g| g.entity_id)
                    {
                        host.hub.entity_event(id, event as i8);
                    }
                }
                MotionEvent::Fall(event) => {
                    let player = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| g.uuid);
                    // The rule's own game event goes out as the player whose
                    // landing it was (`FarmlandBlock.turnToBaseBlock`'s change,
                    // `TurtleEggBlock.decreaseEggs`' destroy): a sculk sensor, a
                    // calibrated sensor and a warden hear it.
                    if let Some(vibration) = self.landing_block_updates(player, &event) {
                        self.as_guest(conn, |game| game.post_landing_vibration(vibration));
                    }
                }
                // PowderSnowBlock.entityInside: every player around sees the
                // snowflake (`ServerLevel.sendParticles`).
                MotionEvent::Snowflake { position, velocity } => {
                    if let Some(host) = self.lan.as_ref() {
                        crate::game::lan_particles::send_snowflake_particle(
                            host,
                            [position.x, position.y, position.z],
                            [velocity.x, velocity.y, velocity.z],
                        );
                    }
                }
                // LivingEntity.baseTick's entity event 67, to every client
                // tracking the guest and its own; this window draws it too.
                MotionEvent::Drown => {
                    if let Some(host) = self.lan.as_ref()
                        && let Some(g) = host.hub.guest(conn)
                    {
                        host.hub
                            .entity_event(g.entity_id, crate::game::combat::DROWN_EVENT);
                    }
                    self.drown_particles(at, [0.0; 3]);
                }
                // ServerPlayer.onInsideBlock: the enter_block trigger, for the
                // player who moved.
                MotionEvent::EnterBlock { state } => {
                    self.as_guest(conn, |game| game.trigger_enter_block(state));
                }
                // HoneyBlock.maybeDoSlideEffects: the sound reaches the
                // others (Player.playSound excludes the player, whose client
                // plays it), the entity event 53 everyone around.
                MotionEvent::HoneySlide {
                    pos,
                    sound,
                    particles,
                } => {
                    if self.world.game_time % 20 == 0 {
                        self.as_guest(conn, |game| game.trigger_honey_slide([pos.x, pos.y, pos.z]));
                    }
                    let Some((id, at)) = self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .map(|g| (g.entity_id, g.position))
                    else {
                        continue;
                    };
                    if sound {
                        let name = "minecraft:block.honey_block.slide";
                        self.audio.play_unshared(name, Some(at), 1.0, 1.0);
                        self.send_level_sound(LevelSound::new(name, at, 1.0, 1.0), Some(conn));
                    }
                    if particles {
                        self.honey_slide_particles(at);
                        if let Some(host) = self.lan.as_ref() {
                            host.hub.entity_event(id, 53);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(v) = marked {
            if mod_api::__private::listened(mod_api::HookId::PlayerVelocity)
                && let Some((uuid, name)) = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guest(conn))
                    .map(|g| (g.uuid, g.name.clone()))
            {
                self.native_player_velocity(uuid, &name, [v.x, v.y, v.z]);
            }
            if let Some(g) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) {
                g.send(&C::SetEntityMotion(play::SetEntityMotion {
                    id: g.entity_id,
                    velocity: [v.x, v.y, v.z],
                }));
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/building_cache.rs"]
mod building_cache_tests;
