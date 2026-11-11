//! Mobs riding mobs: boarding and dismounting, keeping riders on their
//! vehicles, the jockeys that spawn mounted (chicken jockeys, spider
//! jockeys, strider jockeys, zombie horsemen) and the skeleton horse trap.
//! Saving riders under `Passengers` lives in `persist.rs`.
use super::Game;
use horizon_client::mobs::{Species, riders};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use std::collections::HashMap;

/// The mobs riding each vehicle, by the vehicle's id: their indices, in the mobs' order.
pub(super) type Riders = HashMap<u32, Vec<usize>>;

/// The seconds a skeleton horse trap waits for a player: 15 minutes.
const TRAP_LIFETIME: i32 = 18000;
/// `SkeletonTrapGoal`: a player this close springs the trap.
const TRAP_RANGE: f64 = 10.0;
/// Ticks of invulnerability the trap's riders and horses start with.
const TRAP_INVULNERABLE: i32 = 60;

impl Game<'_> {
    /// Resident passengers below an object that never joined cannot be reached
    /// by ServerLevel's recursive passenger tick. Empty on the normal path.
    pub(super) fn mobs_below_off_level(&self) -> std::collections::HashSet<u32> {
        if self.persist.off_level.is_empty() {
            return Default::default();
        }
        let mounts: HashMap<_, _> = self.mob_objects().map(|m| (m.id, m.extra.mount)).collect();
        self.mobs
            .iter()
            .filter_map(|m| {
                let mut mount = m.extra.mount;
                for _ in 0..mounts.len() {
                    let id = mount?;
                    if self.persist.off_level.contains_key(&id) {
                        return Some(m.id);
                    }
                    mount = mounts.get(&id).copied().flatten();
                }
                None
            })
            .collect()
    }

    /// `Entity.startRiding` for two mobs.
    pub(super) fn mount_mob(&mut self, rider: u32, vehicle: u32) {
        self.try_mount_mob(rider, vehicle);
    }
    /// `Entity.startRiding` for two mobs: false when a mod canceled it.
    pub(super) fn try_mount_mob(&mut self, rider: u32, vehicle: u32) -> bool {
        let vehicle = if self.boats.iter().any(|b| b.id == vehicle) {
            super::commands::Target::Boat(vehicle)
        } else if self.carts.iter().any(|c| c.id == vehicle) {
            super::commands::Target::Cart(vehicle)
        } else {
            super::commands::Target::Mob(vehicle)
        };
        self.start_riding_target(super::commands::Target::Mob(rider), vehicle, true)
    }
    /// Riders sit on their vehicles (`Entity.positionRider`), mobs or boats,
    /// and let go when the vehicle dies or is gone.
    pub(super) fn sync_riders(&mut self) {
        if !self.persist.off_level.is_empty() {
            self.sync_retained_riders();
            self.position_riders();
            if self.persist.off_level.values().any(
                |object| matches!(object, crate::game::persist::OffLevel::Mob(mob) if mob.removed),
            ) {
                // Keep a removed vehicle only while cancellation leaves actual
                // references to it; completed dismounts release its ownership.
                self.prune_entity_memory();
            }
            return;
        }
        // Entity.baseTick.
        for m in &mut self.mobs {
            if m.extra.boarding_cooldown > 0 {
                m.extra.boarding_cooldown -= 1;
            }
        }
        let boats: std::collections::HashSet<u32> = self.boats.iter().map(|b| b.id).collect();
        let carts: std::collections::HashSet<u32> = self.carts.iter().map(|c| c.id).collect();
        let cushions: std::collections::HashSet<u32> = self.cushions.iter().map(|c| c.id).collect();
        let tnts: std::collections::HashSet<u32> = self
            .tnts
            .iter()
            .map(|t| t.id)
            .filter(|&id| id != 0)
            .collect();
        let index: HashMap<u32, usize> = self
            .mobs
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        let mut riding: HashMap<u32, (i64, u32)> = HashMap::new();
        for i in 0..self.mobs.len() {
            let Some(v) = self.mobs[i].extra.mount else {
                continue;
            };
            // Not a text display: a server's display mounts come through
            // `remote_set_passengers`, which seats them in `position_riders`.
            let valid = !self.mobs[i].removed
                && (boats.contains(&v)
                    || carts.contains(&v)
                    || tnts.contains(&v)
                    || cushions.contains(&v)
                    || self.seat(v).is_some()
                    || index.get(&v).is_some_and(|&j| !self.mobs[j].removed));
            if valid {
                let place = (self.mobs[i].extra.vehicle_order, self.mobs[i].id);
                riding
                    .entry(v)
                    .and_modify(|first| *first = (*first).min(place))
                    .or_insert(place);
            } else {
                let m = &mut self.mobs[i];
                m.extra.mount = None;
                m.extra.passenger = false;
            }
        }
        for m in &mut self.mobs {
            m.extra.rider = riding.get(&m.id).map(|&(_, id)| id);
        }
        self.position_riders();
    }
    fn sync_retained_riders(&mut self) {
        let invalid: Vec<_> = self
            .mob_objects()
            .filter_map(|m| {
                let v = m.extra.mount?;
                let valid = !m.removed
                    && (self.mob_object(v).is_some_and(|m| !m.removed)
                        || self.boats.iter().any(|b| b.id == v)
                        || self.carts.iter().any(|c| c.id == v)
                        || self.cushions.iter().any(|c| c.id == v)
                        || self.tnts.iter().any(|t| t.id == v && v != 0)
                        || self.seat(v).is_some());
                (!valid).then_some((m.id, v))
            })
            .collect();
        for (id, vehicle) in invalid {
            if self.mob_object(id).and_then(|m| m.extra.mount) != Some(vehicle) {
                continue;
            }
            // Dismount supplies positioning, descendant snapping and the
            // cancellable mount hook. Never overwrite its post-callback edge.
            self.dismount_mob(id);
        }
        let mut first: HashMap<u32, (i64, u32)> = HashMap::new();
        for m in self.mob_objects() {
            if let Some(v) = m.extra.mount {
                let place = (m.extra.vehicle_order, m.id);
                first
                    .entry(v)
                    .and_modify(|p| *p = (*p).min(place))
                    .or_insert(place);
            }
        }
        let ids: Vec<_> = self.mob_objects().map(|m| m.id).collect();
        for id in ids {
            self.mob_object_mut(id).unwrap().extra.rider = first.get(&id).map(|&(_, id)| id);
        }
        let blocked = self.mobs_below_off_level();
        for m in &mut self.mobs {
            if !blocked.contains(&m.id) && m.extra.boarding_cooldown > 0 {
                m.extra.boarding_cooldown -= 1;
            }
        }
    }

    /// The vehicle's attachment for this passenger's actual place in getPassengers.
    /// Shared by ordinary positioning and Entity.teleportPassengers after dismount.
    pub(super) fn mob_passenger_attachment(
        vehicle: &horizon_client::mobs::Mob,
        rider: &horizon_client::mobs::Mob,
        seat: usize,
        count: usize,
    ) -> [f64; 3] {
        if horizon_client::mobs::camel::is_camel(vehicle.species()) {
            horizon_client::mobs::camel::passenger_attachment_at(
                vehicle,
                seat,
                count,
                horizon_client::mobs::drops::is_animal(rider.species()),
            )
        } else if vehicle.species() == Species::HappyGhast {
            riders::happy_ghast_attachment(seat)
        } else {
            riders::passenger_attachment_at(vehicle, seat)
        }
    }

    /// `Entity.positionRider` for every mob passenger: in its boat, then on
    /// its mob. A server's client places them itself too, as the server sends
    /// no passenger's position.
    pub(super) fn position_riders(&mut self) {
        let blocked = self.mobs_below_off_level();
        // Every vehicle's riders in one pass over the mobs, not one pass per
        // boat, minecart and primed TNT. Seating moves the riders but changes
        // no one's vehicle, so it holds for all three.
        let mut riders = Riders::default();
        for (j, m) in self.mobs.iter().enumerate() {
            if blocked.contains(&m.id) {
                continue;
            }
            if let Some(v) = m.extra.mount {
                riders.entry(v).or_default().push(j);
            }
        }
        self.seat_boat_mobs(&riders);
        self.seat_cart_mobs(&riders);
        self.seat_tnt_mobs(&riders);
        self.seat_cushion_mobs(&riders);
        self.seat_static_mobs(&riders);
        let index: HashMap<u32, usize> = self
            .mobs
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        // Riders of riders settle over a few passes.
        for _ in 0..3 {
            for i in 0..self.mobs.len() {
                if blocked.contains(&self.mobs[i].id) {
                    continue;
                }
                let Some(v) = self.mobs[i].extra.mount else {
                    continue;
                };
                let Some(&j) = index.get(&v) else {
                    // Boats, minecarts, primed TNT and cushions seated their
                    // riders above, each in its own seat and from the
                    // vehicle's drawn position; re-seating them here from the
                    // shared seat 0 yanked boat riders between two places.
                    if self.seats_own_riders(v) {
                        continue;
                    }
                    // A mob on a text display: NPC plugins seat their sitting
                    // NPCs on one. The server's position for a passenger
                    // gives way to its seat, as `rideTick` places it.
                    let me = self.remote.as_ref().map(|r| r.entity_id as u32);
                    let own_seat = self.player.as_ref().map(|p| {
                        p.position
                            .add_vector(Vec3::new(0.0, f64::from(p.dimensions.height), 0.0))
                    });
                    let Some(at) = self.passenger_riding_position(v, me, own_seat, &index) else {
                        continue;
                    };
                    let before = self
                        .text_displays
                        .get(&(v as i32))
                        .map(|d| d.seat().1)
                        .unwrap_or_else(|| {
                            let old = self.mobs[i].previous;
                            glam::DVec3::new(
                                old.x,
                                old.y + riders::vehicle_attachment(&self.mobs[i], 0.0),
                                old.z,
                            )
                        });
                    let m = &mut self.mobs[i];
                    // EntityType.TEXT_DISPLAY's box is zero wide.
                    let offset = riders::vehicle_attachment(m, 0.0);
                    // EntityType.TEXT_DISPLAY's box is a point at the seat.
                    let seat = Vec3::new(at.x, at.y, at.z);
                    m.seated(v, offset, Aabb::new(seat, seat));
                    m.position = Vec3::new(at.x, at.y - offset, at.z);
                    m.previous = Vec3::new(before.x, before.y - offset, before.z);
                    m.velocity = Vec3::ZERO;
                    m.fall_distance = 0.0;
                    m.on_ground = false;
                    m.extra.passenger = true;
                    m.ai.nav.stop();
                    continue;
                };
                let vehicle = &self.mobs[j];
                let mut place = (0, 1);
                if horizon_client::mobs::camel::is_camel(vehicle.species()) {
                    // Camel.getPassengerAttachmentPoint: the seat follows the
                    // rider's place in `getPassengers` (load order).
                    let mut aboard: Vec<usize> = (0..self.mobs.len())
                        .filter(|&k| self.mobs[k].extra.mount == Some(v))
                        .collect();
                    let player_aboard = vehicle.rider.is_some() || self.riding == Some(v);
                    let (seat, count) = if player_aboard {
                        let mob_seat = aboard.iter().position(|&k| k == i).unwrap_or(0);
                        (mob_seat + 1, aboard.len() + 1)
                    } else {
                        if let Some(driver) = vehicle.extra.rider
                            && let Some(pos) =
                                aboard.iter().position(|&k| self.mobs[k].id == driver)
                            && pos != 0
                        {
                            let driver_idx = aboard.remove(pos);
                            aboard.insert(0, driver_idx);
                        }
                        let seat = aboard.iter().position(|&k| k == i).unwrap_or(0);
                        (seat, aboard.len())
                    };
                    let (seat, count) = if self.persist.off_level.is_empty() {
                        (seat, count)
                    } else {
                        let seats = self.seated_passengers(v);
                        (
                            seats
                                .iter()
                                .position(|&(_, id, _)| id == self.mobs[i].id)
                                .unwrap_or(seat),
                            seats.len(),
                        )
                    };
                    place = (seat, count);
                }
                if vehicle.species() == horizon_client::mobs::Species::HappyGhast
                    || vehicle.kind.passenger_attachments.len() > 1
                {
                    // Built-in and content seats follow the rider's actual
                    // place in getPassengers, including retained objects.
                    let seat = self.mobs[..i]
                        .iter()
                        .filter(|r| r.extra.mount == Some(v))
                        .count();
                    let seat = if self.persist.off_level.is_empty() {
                        seat
                    } else {
                        self.seated_passengers(v)
                            .iter()
                            .position(|&(_, id, _)| id == self.mobs[i].id)
                            .unwrap_or(seat)
                    };
                    place = (seat, 1);
                }
                let attach =
                    Self::mob_passenger_attachment(vehicle, &self.mobs[i], place.0, place.1);
                let (pos, prev) = (vehicle.position, vehicle.previous);
                let (yaw, prev_yaw) = (vehicle.yaw, vehicle.previous_yaw);
                let (body, prev_body) = (vehicle.body_yaw, vehicle.previous_body_yaw);
                let rider = &self.mobs[i];
                let controlling = riders::controls_vehicle(
                    vehicle,
                    rider,
                    vehicle.rider.is_some() || self.riding == Some(v),
                    self.res
                        .mob_data()
                        .entity_has("minecraft:non_controlling_rider", &rider.kind.id),
                );
                let takes_body = riders::takes_vehicle_body(vehicle, rider, controlling);
                let vehicle_width = vehicle.width();
                let vehicle_box = vehicle.bounds();
                let m = &mut self.mobs[i];
                let offset = riders::vehicle_attachment(m, vehicle_width);
                m.seated(v, offset, vehicle_box);
                m.position = riders::seat(attach, offset, pos, yaw);
                m.previous = riders::seat(attach, offset, prev, prev_yaw);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.on_ground = false;
                if takes_body {
                    m.body_yaw = body;
                    m.previous_body_yaw = prev_body;
                }
                m.extra.vehicle_body = Some((v, prev_body, body));
                m.extra.passenger = true;
                // A rider never walks off on its own.
                m.ai.nav.stop();
            }
        }
        // Displays and interaction boxes riding anything the client tracks,
        // as NPC plugins put a name over a head or a model's bones on a
        // pivot: `Entity.rideTick` -> `positionRider`, the vehicle's passenger
        // attachment (the top of its box in its pose and scale) less the rider's own,
        // which is 0 for both (Display and Interaction are 0 by 0). On the
        // local player, the top of its box in its pose and scale
        // (`Avatar.POSES`).
        let me = self.remote.as_ref().map(|r| r.entity_id as u32);
        let own_seat = self.player.as_ref().map(|p| {
            p.position
                .add_vector(Vec3::new(0.0, f64::from(p.dimensions.height), 0.0))
        });
        // A rider of a rider settles over a few passes, as the mobs above.
        for _ in 0..3 {
            let mut vehicles: Vec<u32> = self
                .text_displays
                .values()
                .filter_map(|d| d.vehicle)
                .chain(self.interactions.values().filter_map(|i| i.vehicle))
                .collect();
            vehicles.sort_unstable();
            vehicles.dedup();
            let seats: HashMap<u32, glam::DVec3> = vehicles
                .into_iter()
                .filter_map(|v| {
                    self.passenger_riding_position(v, me, own_seat, &index)
                        .map(|at| (v, at))
                })
                .collect();
            for display in self.text_displays.values_mut() {
                if let Some(at) = display.vehicle.and_then(|v| seats.get(&v)) {
                    display.ride(*at);
                }
            }
            for interaction in self.interactions.values_mut() {
                if let Some(at) = interaction.vehicle.and_then(|v| seats.get(&v)) {
                    interaction.ride_to(*at);
                }
            }
        }
        // Armor stands riding a mob (a server's name tag hologram): seated
        // like the displays above, so the name follows its vehicle instead of
        // staying where the stand spawned. The stand's own vehicle attachment
        // is the default 0, so its feet sit on the seat. The seat at the
        // start of the tick too (`commonTick` runs before `rideTick`), so the
        // stand is drawn between the two even while the level is frozen and
        // its vehicle is not. Those riding the local player are seated after
        // its move (`seat_own_stands`).
        for i in 0..self.armor_stands.len() {
            let Some(vehicle) = self.armor_stands[i].vehicle else {
                continue;
            };
            let Some(at) = self.passenger_riding_position(vehicle, me, own_seat, &index) else {
                continue;
            };
            let before = index
                .get(&vehicle)
                .map(|&j| {
                    let v = &self.mobs[j];
                    riders::seat(
                        riders::passenger_attachment(v),
                        0.0,
                        v.previous,
                        v.previous_yaw,
                    )
                })
                .unwrap_or(self.armor_stands[i].previous);
            let at = Vec3::new(at.x, at.y, at.z);
            let stand = &mut self.armor_stands[i];
            stand.position = at;
            stand.previous = before;
            stand.velocity = Vec3::ZERO;
        }
    }

    /// Armor stands riding the local player, on the top of its box. Vanilla
    /// ticks the player (`tickNonPassenger`) before its passengers
    /// (`tickPassenger` -> `positionRider`), so a rider's `xo` and `x` are the
    /// seats before and after the same move: call this after the player's.
    pub(super) fn seat_own_stands(&mut self) {
        let (Some(me), Some(p)) = (
            self.remote.as_ref().map(|r| r.entity_id as u32),
            self.player.as_ref(),
        ) else {
            return;
        };
        let up = Vec3::new(0.0, f64::from(p.dimensions.height), 0.0);
        let (at, before) = (
            p.position.add_vector(up),
            self.previous_position.add_vector(up),
        );
        for stand in self
            .armor_stands
            .iter_mut()
            .filter(|a| a.vehicle == Some(me))
        {
            stand.position = at;
            stand.previous = before;
            stand.velocity = Vec3::ZERO;
        }
    }

    /// A vehicle whose `seat_*_mobs` pass places its mob riders itself.
    fn seats_own_riders(&self, vehicle: u32) -> bool {
        self.boats.iter().any(|b| b.id == vehicle)
            || self.carts.iter().any(|c| c.id == vehicle)
            || self.cushions.iter().any(|c| c.id == vehicle)
            || self.tnts.iter().any(|t| t.id == vehicle && t.id != 0)
    }

    /// `Entity.getPassengerRidingPosition` of a vehicle: where the feet of a
    /// passenger whose own vehicle attachment is 0 land.
    pub(super) fn passenger_riding_position(
        &self,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
    ) -> Option<glam::DVec3> {
        self.passenger_riding_position_inner(vehicle, me, own_seat, index, &mut Vec::new())
    }

    fn passenger_riding_position_inner(
        &self,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
        path: &mut Vec<u32>,
    ) -> Option<glam::DVec3> {
        if path.len() >= 64 || path.contains(&vehicle) {
            return None;
        }
        path.push(vehicle);
        if Some(vehicle) == me {
            let at = own_seat?;
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(v) = index
            .get(&vehicle)
            .map(|&j| &self.mobs[j])
            .or_else(|| self.mobs.iter().find(|m| m.id == vehicle))
        {
            let at = riders::seat(riders::passenger_attachment(v), 0.0, v.position, v.yaw);
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(d) = self.text_displays.get(&(vehicle as i32)) {
            return if let Some(parent) = d.vehicle {
                self.passenger_riding_position_inner(parent, me, own_seat, index, path)
            } else {
                Some(d.position())
            };
        }
        if let Some(i) = self.interactions.get(&(vehicle as i32)) {
            if let Some(parent) = i.vehicle {
                return self.passenger_riding_position_inner(parent, me, own_seat, index, path);
            }
            let at = i.position;
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(b) = self.boats.iter().find(|b| b.id == vehicle) {
            let count = self.boat_players(vehicle).max(1) + self.boat_mobs(vehicle).len();
            let at = b.seat(0, count, false, 0.0, false);
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == vehicle) {
            let at = c
                .position
                .add_vector(Vec3::new(0.0, super::minecarts::SEAT, 0.0));
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(a) = self.armor_stands.iter().find(|a| a.id == vehicle) {
            // ArmorStand.getDefaultDimensions: a marker has no box to sit on.
            // The passenger point is the float height widened to double.
            let height = if a.marker {
                0.0
            } else if a.small {
                f64::from(0.9875_f32)
            } else {
                f64::from(1.975_f32)
            };
            let at = if let Some(parent) = a.vehicle {
                self.passenger_riding_position_inner(parent, me, own_seat, index, path)?
            } else {
                glam::DVec3::new(a.position.x, a.position.y, a.position.z)
            };
            return Some(at + glam::DVec3::Y * height);
        }
        // Any other tracked entity sits on its feet; an area effect cloud is
        // 0.5 tall, and ModelEngine parks a model's pivot half a block below
        // its feet for exactly that.
        let remote = self.remote.as_ref()?;
        let (kind, position) = remote.tracked(vehicle as i32)?;
        let position = if let Some(&parent) = remote.passenger_vehicles.get(&vehicle) {
            let at = self.passenger_riding_position_inner(parent, me, own_seat, index, path)?;
            [at.x, at.y, at.z]
        } else {
            position
        };
        let lift = if kind == "minecraft:area_effect_cloud" {
            0.5
        } else {
            0.0
        };
        Some(glam::DVec3::new(
            position[0],
            position[1] + lift,
            position[2],
        ))
    }

    /// `SkeletonHorse.customServerAiStep` and `SkeletonTrapGoal`: a trap
    /// horse is a trap for fifteen minutes; a player within ten blocks springs
    /// it.
    pub(super) fn tick_skeleton_traps(&mut self) {
        // `Level.hasNearbyAlivePlayer`: alive, not a spectator, in the level.
        let players: Vec<_> = self
            .level_players()
            .into_iter()
            .filter(|p| p.alive && !p.spectator)
            .map(|p| p.position)
            .collect();
        let mut sprung = Vec::new();
        let blocked = self.mobs_below_off_level();
        for m in &mut self.mobs {
            if blocked.contains(&m.id) {
                continue;
            }
            if m.species() != Species::SkeletonHorse || !m.extra.trap || !m.alive() {
                continue;
            }
            if !m.persistent {
                m.extra.trap_time += 1;
                if m.extra.trap_time > TRAP_LIFETIME {
                    m.removed = true;
                    continue;
                }
            }
            if players
                .iter()
                .any(|&p| m.distance_sqr(p) < TRAP_RANGE * TRAP_RANGE)
            {
                sprung.push(m.id);
            }
        }
        for id in sprung {
            self.spring_skeleton_trap(id);
        }
    }
    fn spring_skeleton_trap(&mut self, id: u32) {
        let Some(horse) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        horse.extra.trap = false;
        horse.extra.tame = true;
        horse.extra.age = 0;
        let at = [horse.position.x, horse.position.y, horse.position.z];
        self.spawn_bolt(at, true);
        let Some(skeleton) = self.trap_skeleton(at) else {
            return;
        };
        self.mount_mob(skeleton, id);
        for _ in 0..3 {
            let Some(other) = self.spawn_mob(
                "minecraft:skeleton_horse",
                at,
                true,
                super::creatures::SpawnSite::Finalizing(
                    horizon_client::mobs::spawn_rules::Reason::Triggered,
                ),
            ) else {
                continue;
            };
            let (dx, dz) = (
                self.world.random.triangle(0.0, 1.1485),
                self.world.random.triangle(0.0, 1.1485),
            );
            if let Some(h) = self.mobs.iter_mut().find(|m| m.id == other) {
                h.invulnerable_time = TRAP_INVULNERABLE;
                h.extra.tame = true;
                h.extra.age = 0;
                h.velocity = Vec3::new(h.velocity.x + dx, h.velocity.y, h.velocity.z + dz);
            }
            if let Some(rider) = self.trap_skeleton(at) {
                self.mount_mob(rider, other);
            }
        }
    }
    /// `SkeletonTrapGoal.createSkeleton`: an iron helmet unless it already
    /// has a helm.
    fn trap_skeleton(&mut self, at: [f64; 3]) -> Option<u32> {
        let id = self.spawn_mob(
            "minecraft:skeleton",
            at,
            true,
            super::creatures::SpawnSite::Finalizing(
                horizon_client::mobs::spawn_rules::Reason::Triggered,
            ),
        )?;
        let m = self.mobs.iter_mut().find(|m| m.id == id)?;
        m.invulnerable = TRAP_INVULNERABLE;
        if m.extra.head_id().is_none() {
            m.extra.set_slot_id(
                horizon_items::equipment::HEAD,
                Some("minecraft:iron_helmet".into()),
            );
        }
        let finalize = horizon_client::mobs::finalize::item;
        let (weapon, helmet) = (
            finalize(m, horizon_client::mobs::finalize::MAINHAND),
            finalize(m, horizon_client::mobs::finalize::HEAD),
        );
        let special = self.local_difficulty(at).special();
        // `SkeletonTrapGoal.enchant`: the weapon and the helmet, always.
        for (slot, item) in [
            (horizon_client::mobs::gear::MAIN, weapon),
            (horizon_client::mobs::gear::HEAD, helmet),
        ] {
            if let Some(item) = item {
                self.enchant_item(id, slot, &item, special);
            }
        }
        Some(id)
    }
}

#[cfg(test)]
mod off_level_tests {
    use super::*;
    use horizon_client::mobs::Mob;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn dying_riding_edges_do_not_depend_on_unrelated_retained_objects() {
        for retained in [false, true] {
            for dying_vehicle in [false, true] {
                crate::game::native_entity_refs::tests::with_frozen_game(|game| {
                    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                    let mut vehicle = Mob::new(161, kind.clone(), Vec3::ZERO, 0.0);
                    let mut rider = Mob::new(162, kind.clone(), Vec3::ZERO, 0.0);
                    rider.extra.mount = Some(161);
                    if dying_vehicle {
                        vehicle.health = 0.0;
                    } else {
                        rider.health = 0.0;
                    }
                    game.mobs.push(vehicle);
                    game.mobs.push(rider);
                    if retained {
                        game.persist.off_level.insert(
                            169,
                            crate::game::persist::OffLevel::Mob(Mob::new(
                                169,
                                kind,
                                Vec3::ZERO,
                                0.0,
                            )),
                        );
                    }
                    game.sync_riders();
                    assert_eq!(
                        game.mob_object(162).unwrap().extra.mount,
                        Some(161),
                        "retained={retained}, dying_vehicle={dying_vehicle}"
                    );
                });
            }
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn first_rider_order_does_not_depend_on_unrelated_retained_objects() {
        for retained in [false, true] {
            crate::game::native_entity_refs::tests::with_frozen_game(|game| {
                let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let mut vehicle = Mob::new(171, kind.clone(), Vec3::ZERO, 0.0);
                vehicle.extra.rider = Some(172);
                game.mobs.push(vehicle);
                for (id, order) in [(172, 1), (173, 0)] {
                    let mut rider = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                    rider.extra.mount = Some(171);
                    rider.extra.vehicle_order = order;
                    game.mobs.push(rider);
                }
                if retained {
                    game.persist.off_level.insert(
                        179,
                        crate::game::persist::OffLevel::Mob(Mob::new(179, kind, Vec3::ZERO, 0.0)),
                    );
                }
                game.sync_riders();
                assert_eq!(
                    game.mob_object(171).unwrap().extra.rider,
                    Some(173),
                    "retained={retained}"
                );
            });
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn content_seat_positioning_counts_retained_passengers() {
        crate::game::native_entity_refs::tests::with_frozen_game(|game| {
            let mut kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            std::sync::Arc::make_mut(&mut kind).passenger_attachments =
                vec![[1.0, 2.0, 3.0], [-1.0, 4.0, 5.0]];
            game.mobs.push(Mob::new(
                101,
                kind.clone(),
                Vec3::new(10.0, 64.0, 20.0),
                0.0,
            ));
            let mut held = Mob::new(102, kind.clone(), Vec3::ZERO, 0.0);
            held.extra.mount = Some(101);
            held.extra.vehicle_order = 0;
            game.persist
                .off_level
                .insert(102, crate::game::persist::OffLevel::Mob(held));
            let mut resident = Mob::new(103, kind, Vec3::ZERO, 0.0);
            resident.extra.mount = Some(101);
            resident.extra.vehicle_order = 1;
            game.mobs.push(resident);
            game.position_riders();
            let position = game.mob_object(103).unwrap().position;
            assert_eq!(position.x, 9.0);
            assert_eq!(position.z, 25.0);
            assert_eq!(game.mob_object(102).unwrap().position, Vec3::ZERO);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn dismounted_retained_multiseat_vehicle_snaps_each_frozen_passenger() {
        for name in ["minecraft:camel", "minecraft:happy_ghast"] {
            crate::game::native_entity_refs::tests::with_frozen_game(|game| {
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut root = Mob::new(70, pig.clone(), Vec3::new(10.0, 200.0, 20.0), 0.0);
                root.removed = true;
                let mut vehicle = Mob::new(71, kind, root.position, 0.0);
                vehicle.extra.mount = Some(70);
                game.mobs.push(root);
                game.persist
                    .off_level
                    .insert(71, crate::game::persist::OffLevel::Mob(vehicle));
                // Camel.getPassengerAttachmentPoint: two Animals occupy z +0.7/-0.5.
                // EntityTypes.HAPPY_GHAST: front, left, back, right, all at y +4.
                let expected: &[(f64, f64)] = if name == "minecraft:camel" {
                    &[(0.0, 0.7), (0.0, -0.5)]
                } else {
                    &[(0.0, 1.7), (-1.7, 0.0), (0.0, -1.7), (1.7, 0.0)]
                };
                // Reverse storage order: seats follow vehicle_order, not resident order.
                for seat in (0..expected.len()).rev() {
                    let id = 72 + seat as u32;
                    let mut rider = Mob::new(id, pig.clone(), Vec3::ZERO, 0.0);
                    rider.age = 19;
                    rider.extra.mount = Some(71);
                    rider.extra.vehicle_order = seat as i64;
                    if name == "minecraft:happy_ghast" && seat == 2 {
                        game.persist
                            .off_level
                            .insert(id, crate::game::persist::OffLevel::Mob(rider));
                        let mut child = Mob::new(80, pig.clone(), Vec3::ZERO, 0.0);
                        child.extra.mount = Some(id);
                        child.age = 23;
                        game.mobs.push(child);
                    } else {
                        game.mobs.push(rider);
                    }
                }
                game.sync_riders();
                // The normal pass must not overwrite the one-time snap of frozen descendants.
                game.position_riders();
                let vehicle = game.mob_object(71).unwrap();
                assert_eq!(vehicle.extra.mount, None);
                for (seat, &(x, z)) in expected.iter().enumerate() {
                    let rider = game.mob_object(72 + seat as u32).unwrap();
                    assert!((rider.position.x - vehicle.position.x - x).abs() < 1e-6);
                    assert!((rider.position.z - vehicle.position.z - z).abs() < 1e-6);
                    assert_eq!(rider.previous, rider.position);
                    assert_eq!(rider.age, 19);
                    if name == "minecraft:happy_ghast" {
                        assert!((rider.position.y - vehicle.position.y - 4.0).abs() < 1e-6);
                    }
                }
                if name == "minecraft:happy_ghast" {
                    let parent = game.mob_object(74).unwrap();
                    let child = game.mob_object(80).unwrap();
                    assert_eq!(child.position.x, parent.position.x);
                    assert_eq!(child.position.z, parent.position.z);
                    assert!((child.position.y - parent.position.y - 0.86875).abs() < 1e-6);
                    assert_eq!(child.age, 23);
                }
            });
        }
    }

    struct CountCanceledDismount(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CountCanceledDismount {
        fn entity_mount(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn destroyed_resident_vehicles_notify_cancellation_once_then_safely_unlink() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        for cart in [false, true] {
            let cancel = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "toggle_dismount",
                Box::new(CountCanceledDismount(cancel.clone())),
            )]
            .into();
            mod_api::testing::with_mods(mods, || {
                crate::game::native_entity_refs::tests::with_frozen_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let at = Vec3::new(10.0, 200.0, 20.0);
                    if cart {
                        game.carts.push(super::super::minecarts::Minecart::new(
                            71,
                            super::super::minecarts::CartKind::Rideable,
                            at,
                        ));
                    } else {
                        game.boats
                            .push(super::super::boats::Boat::new(71, "oak".into(), at, 0.0));
                    }
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut rider = Mob::new(72, pig.clone(), Vec3::new(5.0, 190.0, 6.0), 0.0);
                    rider.extra.mount = Some(71);
                    game.persist
                        .off_level
                        .insert(72, super::super::persist::OffLevel::Mob(rider));
                    let mut child = Mob::new(73, pig, Vec3::ZERO, 0.0);
                    child.extra.mount = Some(72);
                    child.age = 19;
                    game.mobs.push(child);
                    if cart {
                        game.hurt_cart(0, 5.0, true, false);
                    } else {
                        assert!(game.hurt_boat(0, 5.0, true));
                    }
                    assert_eq!(cancel.load(Ordering::Relaxed), 1);
                    game.sync_riders();
                    let rider = game.mob_object(72).unwrap();
                    assert_eq!(rider.extra.mount, None);
                    assert_eq!(rider.position, Vec3::new(5.0, 200.0, 6.0));
                    let child = game.mob_object(73).unwrap();
                    assert!((child.position.y - 200.86875).abs() < 1e-6);
                    assert_eq!(child.age, 19);
                    game.prune_entity_memory();
                    assert!(!game.persist.off_level.contains_key(&71));
                    assert!(game.persist.off_level.contains_key(&72));
                    assert!(game.mobs.iter().any(|m| m.id == 73));
                    assert!(!game.mobs.iter().any(|m| m.id == 72));
                    assert_eq!(cancel.load(Ordering::Relaxed), 1);
                });
            });
        }
    }

    struct CancelDismount;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelDismount {
        fn entity_mount(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            assert!(!event.mounting);
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn invalid_retained_mount_uses_cancellable_dismount() {
        for cancel in [false, true] {
            let mods: mod_api::Mods = if cancel {
                vec![mod_api::ModEntry::new(
                    "cancel_dismount",
                    Box::new(CancelDismount),
                )]
                .into()
            } else {
                Vec::new().into()
            };
            mod_api::testing::with_mods(mods, || {
                crate::game::native_entity_refs::tests::with_frozen_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut root = Mob::new(71, pig.clone(), Vec3::new(1.0, 200.0, 0.5), 0.0);
                    root.removed = true;
                    root.extra.rider = Some(72);
                    let mut rider = Mob::new(72, pig.clone(), Vec3::new(1.0, 190.0, 0.5), 0.0);
                    rider.extra.mount = Some(71);
                    rider.extra.passenger = true;
                    let mut child = Mob::new(73, pig, rider.position, 0.0);
                    child.extra.mount = Some(72);
                    child.age = 19;
                    game.mobs.push(child);
                    game.mobs.push(root);
                    game.persist
                        .off_level
                        .insert(72, crate::game::persist::OffLevel::Mob(rider));
                    game.sync_riders();
                    let rider = game.mob_object(72).unwrap();
                    if cancel {
                        assert_eq!(rider.extra.mount, Some(71));
                        assert!(rider.extra.passenger);
                        assert_eq!(rider.position.y, 190.0);
                        assert_eq!(game.mob_object(71).unwrap().extra.rider, Some(72));
                        assert_eq!(game.mob_object(73).unwrap().position.y, 190.0);
                    } else {
                        assert_eq!(rider.extra.mount, None);
                        assert!(!rider.extra.passenger);
                        assert_eq!(
                            rider.extra.boarding_cooldown,
                            super::super::boat_passengers::BOARDING_COOLDOWN
                        );
                        assert_eq!(rider.position.y, 200.0);
                        assert_eq!(game.mob_object(71).unwrap().extra.rider, None);
                        assert!(game.mob_object(73).unwrap().position.y > 200.0);
                    }
                    let child = game.mob_object(73).unwrap();
                    assert_eq!(child.age, 19);
                    assert_eq!(child.extra.mount, Some(72));
                });
            });
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn dying_objects_keep_passenger_links_until_removal() {
        crate::game::native_entity_refs::tests::with_frozen_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut root = Mob::new(71, pig.clone(), Vec3::new(1.0, 200.0, 0.5), 0.0);
            root.health = 0.0;
            let mut parent = Mob::new(72, pig.clone(), root.position, 0.0);
            parent.extra.mount = Some(71);
            let mut child = Mob::new(73, pig, root.position, 0.0);
            child.extra.mount = Some(72);
            game.mobs.push(root);
            game.mobs.push(child);
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(parent));
            for dying_passenger in [false, true] {
                if dying_passenger {
                    game.mob_object_mut(72).unwrap().health = 0.0;
                }
                game.sync_riders();
                assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                assert_eq!(game.mob_object(71).unwrap().extra.rider, Some(72));
                assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(72));
                assert_eq!(game.mob_object(72).unwrap().extra.rider, Some(73));
                assert_eq!(game.mob_object(73).unwrap().position.y, 200.0);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn synchronization_preserves_retained_controller_and_frozen_descendant() {
        crate::game::native_entity_refs::tests::with_frozen_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut root = Mob::new(71, pig.clone(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            root.extra.rider = Some(72);
            let mut parent = Mob::new(72, pig.clone(), root.position, 0.0);
            parent.extra.mount = Some(71);
            parent.extra.rider = Some(73);
            let mut child = Mob::new(73, pig, Vec3::new(7.0, 90.0, 4.0), 0.0);
            child.extra.mount = Some(72);
            child.extra.passenger = true;
            child.extra.boarding_cooldown = 12;
            let before = child.position;
            game.mobs.push(root);
            game.mobs.push(child);
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(parent));
            game.sync_riders();
            assert_eq!(game.mob_object(71).unwrap().extra.rider, Some(72));
            assert_eq!(game.mob_object(72).unwrap().extra.rider, Some(73));
            let child = game.mob_object(73).unwrap();
            assert_eq!(child.extra.mount, Some(72));
            assert!(child.extra.passenger);
            assert_eq!(child.extra.boarding_cooldown, 12);
            assert_eq!(child.position, before);
        });
    }
}

#[cfg(test)]
mod nested_vehicle_discard_tests {
    use super::*;
    use horizon_client::mobs::Mob;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct DiscardVehicleOnDismount(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DiscardVehicleOnDismount {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            assert!(!event.mounting);
            assert_eq!(event.entity, 72);
            self.0.fetch_add(1, Ordering::Relaxed);
            // The nested notification sees the root already logically removed.
            // Do not recurse deliberately or hold a lock across a host call.
            if let Some(vehicle) = host.entity(event.vehicle) {
                assert!(vehicle.discard(host));
            }
            mod_api::Flow::Cancel
        }
    }
    fn discard_during_dismount(cart: bool) {
        let count = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "discard_vehicle_on_dismount",
            Box::new(DiscardVehicleOnDismount(count.clone())),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_frozen_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let at = Vec3::new(10.0, 200.0, 20.0);
                if cart {
                    game.carts.push(super::super::minecarts::Minecart::new(
                        71,
                        super::super::minecarts::CartKind::Rideable,
                        at,
                    ));
                } else {
                    game.boats
                        .push(super::super::boats::Boat::new(71, "oak".into(), at, 0.0));
                }
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut rider = Mob::new(72, kind.clone(), Vec3::new(2.0, 190.0, 3.0), 0.0);
                rider.extra.mount = Some(71);
                game.mobs.push(rider);
                let mut child = Mob::new(73, kind, Vec3::ZERO, 0.0);
                child.extra.mount = Some(72);
                child.age = 19;
                game.persist
                    .off_level
                    .insert(73, super::super::persist::OffLevel::Mob(child));
                let rider = game.entity(72).unwrap();
                game.entity_stop_riding(rider);
                assert_eq!(
                    count.load(Ordering::Relaxed),
                    1,
                    "nested removal must not dispatch the same in-flight dismount twice"
                );
                assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                assert_eq!(
                    game.mob_object(72).unwrap().position,
                    Vec3::new(2.0, 200.0, 3.0)
                );
                let child = game.mob_object(73).unwrap();
                assert_eq!(child.extra.mount, Some(72));
                assert_eq!(child.position.x, 2.0);
                assert_eq!(child.position.z, 3.0);
                assert!(child.position.y >= 200.0);
                assert_eq!(child.age, 19);
                assert!(game.entity(73).is_none());
                game.native_removals_flush();
                assert_eq!(
                    count.load(Ordering::Relaxed),
                    1,
                    "deferred sweep must not notify again"
                );
                assert!(game.entity(71).is_none());
                assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
            });
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn nested_vehicle_discard_notifies_a_boat_dismount_once() {
        discard_during_dismount(false);
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn nested_vehicle_discard_notifies_a_cart_dismount_once() {
        discard_during_dismount(true);
    }
    fn with_vehicle_tree(cart: bool, rejected: bool, test: impl FnOnce(&mut Game<'_>)) {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.options.game_dir = Some(
                std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
                    .join(format!("guard-{:?}", std::thread::current().id())),
            );
            game.world_root = game.options.game_dir.as_ref().unwrap().join("world");
            let at = Vec3::new(10.0, 200.0, 20.0);
            if cart {
                game.carts.push(super::super::minecarts::Minecart::new(
                    71,
                    super::super::minecarts::CartKind::Rideable,
                    at,
                ));
            } else {
                game.boats
                    .push(super::super::boats::Boat::new(71, "oak".into(), at, 0.0));
            }
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut rider = Mob::new(72, kind.clone(), Vec3::new(2.0, 190.0, 3.0), 0.0);
            rider.extra.mount = Some(71);
            if rejected {
                game.persist
                    .off_level
                    .insert(72, super::super::persist::OffLevel::Mob(rider));
            } else {
                game.mobs.push(rider);
            }
            let mut child = Mob::new(73, kind, Vec3::ZERO, 0.0);
            child.extra.mount = Some(72);
            child.age = 19;
            game.mobs.push(child);
            test(game);
        });
    }
    struct CancelLive {
        count: Arc<AtomicUsize>,
        cross_level: bool,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelLive {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            assert!(!event.mounting);
            assert_eq!(event.entity, 72);
            let n = self.count.fetch_add(1, Ordering::Relaxed);
            if self.cross_level {
                assert!(host.commands().run_command(&format!(
                    "execute in minecraft:the_nether run worldborder set {}",
                    37 + n
                )));
                assert!(
                    host.entity(72).is_some(),
                    "the source level is restored inside the callback"
                );
            }
            mod_api::Flow::Cancel
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn kill_and_discard_notify_rejected_edges_once_without_changing_membership() {
        for cart in [false, true] {
            for discard in [false, true] {
                let count = Arc::new(AtomicUsize::new(0));
                let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                    "cancel_cleanup",
                    Box::new(CancelLive {
                        count: count.clone(),
                        cross_level: false,
                    }),
                )]
                .into();
                mod_api::testing::with_mods(mods, || {
                    with_vehicle_tree(cart, true, |game| {
                        if discard {
                            assert!(game.entity(71).unwrap().discard(game));
                        } else {
                            game.kill_target_in_level(if cart {
                                super::super::commands::Target::Cart(71)
                            } else {
                                super::super::commands::Target::Boat(71)
                            });
                        }
                        game.native_removals_flush();
                        assert_eq!(count.load(Ordering::Relaxed), 1);
                        assert!(game.entity(71).is_none());
                        assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                        assert_eq!(
                            game.mob_object(72).unwrap().position,
                            Vec3::new(2.0, 200.0, 3.0)
                        );
                        assert!(game.entity(72).is_none());
                        assert!(game.persist.off_level.contains_key(&72));
                        let child = game.mob_object(73).unwrap();
                        assert_eq!(child.extra.mount, Some(72));
                        assert_eq!(child.position.x, 2.0);
                        assert_eq!(child.position.z, 3.0);
                        assert!(child.position.y >= 200.0);
                        assert_eq!(child.age, 19);
                        assert!(game.entity(73).is_some());
                        assert!(game.persist.mount_notifications.is_empty());
                        game.native_removals_flush();
                        assert_eq!(count.load(Ordering::Relaxed), 1);
                    });
                });
            }
        }
    }
    struct ReparentLater(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReparentLater {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.entity == 73 {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            if !event.mounting && event.entity == 72 {
                let later = host.entity(73).unwrap();
                let replacement = host.entity(74).unwrap();
                assert!(later.start_riding(host, replacement, true));
                return mod_api::Flow::Cancel;
            }
            mod_api::Flow::Continue
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn destruction_preserves_later_rider_reparenting_without_an_extra_notification() {
        for cart in [false, true] {
            let count = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "reparent_cleanup",
                Box::new(ReparentLater(count.clone())),
            )]
            .into();
            mod_api::testing::with_mods(mods, || {
                with_vehicle_tree(cart, false, |game| {
                    game.mob_object_mut(72).unwrap().extra.vehicle_order = 1;
                    let later = game.mob_object_mut(73).unwrap();
                    later.extra.mount = Some(71);
                    later.extra.vehicle_order = 2;
                    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                    game.mobs.push(Mob::new(74, kind, Vec3::ZERO, 0.0));
                    game.kill_target_in_level(if cart {
                        super::super::commands::Target::Cart(71)
                    } else {
                        super::super::commands::Target::Boat(71)
                    });
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
                    assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                    assert_eq!(
                        count.load(Ordering::Relaxed),
                        1,
                        "only deliberate reparenting dismounts the later rider"
                    );
                    game.native_removals_flush();
                    assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                    assert_eq!(count.load(Ordering::Relaxed), 1);
                    assert!(game.persist.mount_notifications.is_empty());
                });
            });
        }
    }
    fn live_cancels(cross_level: bool) {
        let count = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "live_cancel",
            Box::new(CancelLive {
                count: count.clone(),
                cross_level,
            }),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            with_vehicle_tree(false, false, |game| {
                if cross_level {
                    // The console service requires an active server hub.
                    super::super::native_player_death::tests::add_guest(game);
                    game.build_registries();
                    game.set_allow_commands(true);
                    assert!(game.run_command("worldborder set 1000"));
                }
                let rider = game.entity(72).unwrap();
                for n in 1..=2 {
                    assert!(!game.entity_stop_riding(rider));
                    assert_eq!(count.load(Ordering::Relaxed), n);
                    assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(71));
                    assert_eq!(
                        game.mob_object(72).unwrap().position,
                        Vec3::new(2.0, 190.0, 3.0)
                    );
                    assert!(game.persist.mount_notifications.is_empty());
                    assert_eq!(game.dim, super::super::Dim::Overworld);
                }
                if cross_level {
                    assert_eq!(game.border.size(), 1000.0);
                    assert_eq!(
                        game.levels
                            .parked_border(super::super::Dim::Nether)
                            .unwrap()
                            .size(),
                        38.0
                    );
                    game.enter_level(super::super::Dim::Nether).unwrap();
                    assert!(game.persist.mount_notifications.is_empty());
                    game.enter_level(super::super::Dim::Overworld).unwrap();
                    assert!(game.persist.mount_notifications.is_empty());
                }
            });
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn independent_live_cancellations_both_notify_and_retain_the_relationship() {
        live_cancels(false);
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn scoped_cross_level_callbacks_restore_context_and_clear_notification_keys() {
        live_cancels(true);
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn review_cycle_through_retained_object_is_refused() {
        with_vehicle_tree(false, true, |game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(74, kind, Vec3::ZERO, 0.0));
            game.mob_object_mut(72).unwrap().extra.mount = Some(74);
            assert!(
                !game.start_riding_target(
                    super::super::commands::Target::Mob(74),
                    super::super::commands::Target::Mob(73),
                    true
                ),
                "cycle through retained object must be rejected"
            );
        });
    }
    struct ReparentSelf(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReparentSelf {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if !event.mounting && event.entity == 72 && event.vehicle == 71 {
                self.0.fetch_add(1, Ordering::Relaxed);
                host.entity(71).unwrap().discard(host);
                let rider = host.entity(72).unwrap();
                let replacement = host.entity(74).unwrap();
                assert!(rider.start_riding(host, replacement, true));
            }
            mod_api::Flow::Continue
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn dismount_preserves_its_own_riders_callback_replacement() {
        let count = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reparent_self",
            Box::new(ReparentSelf(count.clone())),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            with_vehicle_tree(false, false, |game| {
                let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs.push(Mob::new(74, kind, Vec3::ZERO, 0.0));
                game.dismount_mob(72);
                assert_eq!(game.mob_object(72).unwrap().extra.mount, Some(74));
                assert_eq!(game.mob_passenger_ids(74), vec![72]);
                assert!(
                    !game
                        .seated_passengers(71)
                        .iter()
                        .any(|(_, id, _)| *id == 72)
                );
                assert_eq!(count.load(Ordering::Relaxed), 1);
                assert!(game.persist.mount_notifications.is_empty());
            })
        });
    }
    struct ReviewKillOnMount;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ReviewKillOnMount {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            if event.mounting {
                host.entity(event.vehicle).unwrap().discard(host);
            }
            mod_api::Flow::Continue
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn review_mount_callback_must_not_link_removed_vehicle() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "review",
            Box::new(ReviewKillOnMount),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            with_vehicle_tree(false, false, |game| {
                game.mob_object_mut(72).unwrap().extra.mount = None;
                let ok = game.start_riding_target(
                    super::super::commands::Target::Mob(72),
                    super::super::commands::Target::Boat(71),
                    true,
                );
                assert!(!ok, "mount must refuse when callback removes vehicle");
                assert_eq!(game.mob_object(72).unwrap().extra.mount, None);
            })
        });
    }
}

impl Game<'_> {
    pub(super) fn dismount_killed_boat(&mut self, id: u32) {
        if let Some(y) = self.boats.iter().find(|b| b.id == id).map(|b| b.position.y) {
            self.dismount_destroyed_vehicle_mobs(id, y);
        }
    }
}

impl Game<'_> {
    pub(super) fn dismount_killed_cart(&mut self, id: u32) {
        if let Some(y) = self.carts.iter().find(|c| c.id == id).map(|c| c.position.y) {
            self.dismount_destroyed_vehicle_mobs(id, y);
        }
    }
}

impl Game<'_> {
    pub(super) fn dismount_cart_mob_at(&mut self, rider: u32, y: f64) {
        let Some(mob) = self.mob_object(rider) else {
            return;
        };
        let at = self.removed_vehicle_dismount_point(
            mob.position,
            f64::from(mob.width()),
            f64::from(mob.height()),
            y,
        );
        self.dismount_mob(rider);
        if let Some(mob) = self.mob_object_mut(rider)
            && mob.extra.mount.is_none()
        {
            mob.position = at;
            mob.previous = at;
            self.snap_mob_passengers(rider);
        }
    }
}

#[cfg(test)]
mod content_seat_tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources"]
    fn content_seats_keep_independent_offsets() {
        super::super::native_entity_refs::tests::with_game(|game| {
            use std::sync::Arc;
            let mut kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            Arc::make_mut(&mut kind).passenger_attachments =
                vec![[1.0, 2.0, 3.0], [-1.0, 4.0, 5.0]];
            let vehicle = horizon_client::mobs::Mob::new(1, kind, Vec3::ZERO, 0.0);
            assert_eq!(
                riders::passenger_attachment_at(&vehicle, 0),
                [1.0, 2.0, 3.0]
            );
            assert_eq!(
                riders::passenger_attachment_at(&vehicle, 1),
                [-1.0, 4.0, 5.0]
            );
        });
    }
}
