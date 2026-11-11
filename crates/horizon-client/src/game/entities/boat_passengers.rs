//! Boats and the entities around them: what a boat runs into
//! (`AbstractBoat.canVehicleCollide`), the pushes between a boat and what
//! touches it (`AbstractBoat.push`, `LivingEntity.pushEntities`), the mobs
//! it picks up (the end of `AbstractBoat.tick`), where they sit
//! (`positionRider`, `clampRotation`) and how they get out.
//! Saving them under the boat's `Passengers` lives in `persist.rs`.
use super::{Game, boats::Boat, lan_mobs::guest_bounds};
use horizon_client::{
    camera::wrap_degrees,
    mobs::{BoxReach, Mob, Species, Who, drops::is_animal, equine, riders},
    scoreboard::pushable_by,
};
use horizon_core::geometry::{Aabb, Vec3};

/// `Entity.boardingCooldown` once a mob leaves its boat.
pub(super) const BOARDING_COOLDOWN: i32 = 60;
/// `Player.DEFAULT_VEHICLE_ATTACHMENT`: how far below the seat a player's
/// feet are.
const PLAYER_RIDING_OFFSET: f64 = 0.6;

/// `Entity.canRide` for a mob species: `Warden` and `WitherBoss` override it
/// to return false, so no vehicle ever takes them aboard.
pub(super) fn species_can_ride(species: Species) -> bool {
    !matches!(species, Species::Warden | Species::Wither)
}

/// `Entity.isVehicle`: a player or a mob rides it.
fn carries(m: &Mob) -> bool {
    m.rider.is_some() || m.extra.rider.is_some()
}
/// `isPushable`, as `pushableBy` and `canVehicleCollide` ask it:
/// `Mob::pushable` plus the shulkers it leaves out (they take no push, but
/// count), less a ridden horse (`AbstractHorse.isPushable`).
fn pushable(m: &Mob) -> bool {
    !m.removed
        && (m.pushable() || m.species() == Species::Shulker && m.alive())
        && !(equine::is_equine(m.species()) && carries(m))
}
/// Whether `Entity.push` moves the mob: it is pushable and carries no one.
fn moved_by_push(m: &Mob) -> bool {
    !m.removed && m.pushable() && !carries(m)
}
/// `Entity.push(Entity)`: the push `other` gets from `this` (which takes the
/// opposite one), or None when they stand on nearly the same spot.
pub(super) fn push_impulse(this: Vec3, other: Vec3) -> Option<Vec3> {
    let (mut xa, mut za) = (other.x - this.x, other.z - this.z);
    let mut dd = xa.abs().max(za.abs());
    if dd < f64::from(0.01_f32) {
        return None;
    }
    dd = dd.sqrt();
    xa /= dd;
    za /= dd;
    let pow = (1.0 / dd).min(1.0);
    let k = f64::from(0.05_f32);
    Some(Vec3::new(xa * pow * k, 0.0, za * pow * k))
}
/// `AbstractBoat.push`: another boat is pushed while it is lower than this
/// boat's top, anything else while its feet are no higher than the boat's.
pub(super) fn boat_pushes(boat: Aabb, other: Aabb, other_is_boat: bool) -> bool {
    if other_is_boat {
        other.min_position().y < boat.max_position().y
    } else {
        other.min_position().y <= boat.min_position().y
    }
}

impl Game<'_> {
    /// Starts or ends a stretch of vehicle ticks (`tick_boats`, `tick_carts`): while one runs,
    /// a vehicle's box query tests the mobs' positions against their [`BoxReach`] first.
    /// Mods may grow or rescale a mob between queries, so only an unmodded batch reuses the snapshot.
    /// A mob joining or leaving works it out again.
    pub(super) fn vehicles_ticking(&self, on: bool) {
        self.vehicle_mob_reach.set(
            (on && mod_api::__private::mods().is_empty())
                .then(|| (self.mobs.len(), BoxReach::of(&self.mobs))),
        );
    }
    /// A vehicle picked a mob up: a shulker turns its lid up (`Shulker.startRiding`), which
    /// changes its box, so the next query works the [`BoxReach`] out again.
    pub(super) fn vehicle_picked_up(&self) {
        if let Some((_, reach)) = self.vehicle_mob_reach.get() {
            self.vehicle_mob_reach.set(Some((usize::MAX, reach)));
        }
    }
    /// The mobs' [`BoxReach`] while vehicles tick, none otherwise.
    pub(super) fn vehicle_mob_reach(&self) -> Option<BoxReach> {
        let (count, reach) = self.vehicle_mob_reach.get()?;
        if count == self.mobs.len() {
            return Some(reach);
        }
        let reach = BoxReach::of(&self.mobs);
        self.vehicle_mob_reach.set(Some((self.mobs.len(), reach)));
        Some(reach)
    }
    /// The players in boat `id`: the window's player (or the server player
    /// acting as it) and the server's other players.
    pub(super) fn boat_players(&self, id: u32) -> usize {
        let guests = self.lan.as_ref().map_or(0, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| Some(g.conn) != self.lan_acting && g.vehicle == Some(id))
                .count()
        });
        usize::from(self.riding == Some(id)) + guests
    }
    /// The mobs in boat `id`, in their seat order after the players
    /// (`Entity.addPassenger` puts a player first).
    pub(super) fn boat_mobs(&self, id: u32) -> Vec<usize> {
        (0..self.mobs.len())
            .filter(|&j| {
                let m = &self.mobs[j];
                m.extra.mount == Some(id) && !m.removed && m.alive()
            })
            .collect()
    }
    /// `getPassengers().size()` of boat `id`.
    pub(super) fn boat_passengers(&self, id: u32) -> usize {
        self.boat_players(id)
            + self
                .mob_objects()
                .filter(|m| m.extra.mount == Some(id) && !m.removed)
                .count()
    }
    /// `AbstractBoat.canAddPassenger`: a seat is free and the boat is not
    /// under water.
    pub(super) fn boat_has_room(&self, i: usize) -> bool {
        let b = &self.boats[i];
        self.boat_passengers(b.id) < b.max_passengers() && !b.under_water()
    }
    /// Where the player in boat `i` sits: in the middle alone, in front of a
    /// mob.
    pub(super) fn player_boat_seat(&self, i: usize) -> Vec3 {
        let b = &self.boats[i];
        let players = self.boat_players(b.id);
        let count = if self.persist.off_level.is_empty() {
            players.max(1) + self.boat_mobs(b.id).len()
        } else {
            self.boat_passengers(b.id) + usize::from(players == 0)
        };
        b.seat(0, count, false, PLAYER_RIDING_OFFSET, false)
    }
    /// `getRootVehicle` of mob `j`: the mob or boat at the bottom of what it
    /// rides, None when it rides nothing.
    pub(super) fn mob_root_vehicle(&self, mut j: usize) -> Option<u32> {
        let mut root = None;
        for _ in 0..8 {
            let Some(v) = self.mobs[j].extra.mount else {
                break;
            };
            root = Some(v);
            match self.mobs.iter().position(|m| m.id == v) {
                Some(k) => j = k,
                None => break,
            }
        }
        root
    }
    /// The players outside boat `id` that push and block it (living, not
    /// spectators), with the window player's team.
    fn players_outside(&self, id: u32) -> Vec<(Aabb, bool)> {
        let mut out = Vec::new();
        if let Some(p) = self.player.as_ref()
            && self.riding != Some(id)
            && !self.spectator()
            && !p.health.dead_or_dying()
        {
            out.push((p.bounds(), true));
        }
        if let Some(host) = self.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| {
                Some(g.conn) != self.lan_acting
                    && host.hub.here(g)
                    && g.loaded
                    && !g.dead
                    && !g.spectator()
                    && g.vehicle != Some(id)
            }) {
                out.push((guest_bounds(g), false));
            }
        }
        out
    }
    /// `getEntityCollisions` of boat `id` within `area`, with
    /// `AbstractBoat.canVehicleCollide`: the other boats, minecarts,
    /// shulkers and pushable mobs and players, but not its own passengers.
    pub(super) fn boat_obstacles(&self, id: u32, area: Aabb) -> Vec<Aabb> {
        let reach = self.vehicle_mob_reach();
        let mut out: Vec<Aabb> = (0..self.mobs.len())
            .filter(|&j| {
                let m = &self.mobs[j];
                // The box first: it turns away nearly every mob of the level.
                reach.is_none_or(|r| r.may_meet(m, area))
                    && m.bounds().intersects(area)
                    && pushable(m)
                    && (m.extra.mount.is_none() || self.mob_root_vehicle(j) != Some(id))
            })
            .map(|j| self.mobs[j].bounds())
            .collect();
        out.extend(
            self.boats
                .iter()
                .filter(|b| b.id != id)
                .map(Boat::bounds)
                .chain(self.carts.iter().map(|c| c.bounds()))
                .chain(self.players_outside(id).into_iter().map(|(bb, _)| bb))
                .filter(|bb| bb.intersects(area)),
        );
        out
    }
    /// What boat `b` may run into this tick.
    pub(super) fn boat_move_obstacles(&self, b: &Boat) -> Vec<Aabb> {
        let reach = b.velocity.length() + 1.0;
        self.boat_obstacles(b.id, b.bounds().inflate(Vec3::new(reach, reach, reach)))
    }
    /// `noEntityCollision(boat, box)` for a new boat (`BoatItem.use`).
    pub(super) fn boat_entity_collision(&self, bb: Aabb) -> bool {
        // EntityGetter.getEntityCollisions: the box grown by 1e-7.
        let area = bb.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        !self.boat_obstacles(u32::MAX, area).is_empty()
    }

    /// The end of `AbstractBoat.tick` on the server for boat `i`: each
    /// entity it touches gets in (a mob, while no player steers) or is
    /// pushed apart from it, and `LivingEntity.pushEntities` of each mob and
    /// player touching it pushes the two apart once more. Only the window's
    /// own player is moved here: a server's players are pushed by their
    /// clients, which tick the boats near them (`remote_pushes`).
    pub(super) fn boat_contacts(&mut self, i: usize) {
        let (id, bb, max) = {
            let b = &self.boats[i];
            (b.id, b.bounds(), b.max_passengers())
        };
        let area = bb.inflate(Vec3::new(
            f64::from(0.2_f32),
            f64::from(-0.01_f32),
            f64::from(0.2_f32),
        ));
        let mut count = self.boat_passengers(id);
        // getControllingPassenger is a player: one rides it (players sit first).
        let pick_up = self.boat_players(id) == 0;
        // canAddPassenger, which startRiding asks after the checks below.
        let room = !self.boats[i].under_water();
        // The box first: it turns away nearly every mob of the level, and the
        // teams are only worked out when something touches the boat.
        let reach = self.vehicle_mob_reach();
        let sweep = area.union_bounds(bb);
        let near: Vec<usize> = (0..self.mobs.len())
            .filter(|&j| {
                if reach.is_some_and(|r| !r.may_meet(&self.mobs[j], sweep)) {
                    return false;
                }
                let mb = self.mobs[j].bounds();
                mb.intersects(area) || mb.intersects(bb)
            })
            .collect();
        let outside = self.players_outside(id);
        let touching = !near.is_empty()
            || outside
                .iter()
                .any(|(pb, _)| pb.intersects(area) || pb.intersects(bb));
        // (mob, touched by the boat's sweep, its own pushEntities reaches the boat)
        let teams = if touching {
            self.mob_teams()
        } else {
            Default::default()
        };
        let mobs: Vec<(u32, bool, bool)> = near
            .into_iter()
            .filter_map(|j| {
                let m = &self.mobs[j];
                if !pushable(m) || m.extra.mount.is_some() && self.mob_root_vehicle(j) == Some(id) {
                    return None;
                }
                let mb = m.bounds();
                let team = teams.of(Who::Mob(m.id));
                let touched = mb.intersects(area) && pushable_by(None, team, true);
                // Bats push nothing (Bat.pushEntities).
                let pushes = m.species() != Species::Bat
                    && mb.intersects(bb)
                    && pushable_by(team, None, true);
                (touched || pushes).then_some((m.id, touched, pushes))
            })
            .collect();
        let player_team = teams.of(Who::Player);
        let players: Vec<(Aabb, bool, bool, bool)> = outside
            .into_iter()
            .map(|(pb, own)| {
                let team = if own { player_team } else { None };
                (
                    pb,
                    own,
                    pb.intersects(area) && pushable_by(None, team, true),
                    pb.intersects(bb) && pushable_by(team, None, true),
                )
            })
            .collect();
        drop(teams);
        for (mob, touched, pushes) in mobs {
            // By id: a mod's mount hook may have removed mobs since the list
            // was made, which moves the others' indices.
            let Some(j) = self.mobs.iter().position(|m| m.id == mob) else {
                continue;
            };
            if touched {
                let m = &self.mobs[j];
                let boards = pick_up
                    && count < max
                    && m.extra.mount.is_none()
                    && m.width() < super::boats::WIDTH as f32
                    && !self
                        .res
                        .mob_data()
                        .entity_has("minecraft:cannot_be_pushed_onto_boats", &m.kind.id);
                if boards {
                    // startRiding fails quietly (no push) while the mob's
                    // boarding cooldown runs (Entity.canRide) or the boat
                    // is under water, or when a mod cancels it
                    // (EntityMountEvent).
                    let ready = m.extra.boarding_cooldown <= 0 && species_can_ride(m.species());
                    if room
                        && ready
                        && self.native_mob_may_ride(mob, id, true)
                        && let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob)
                    {
                        horizon_client::mobs::shulker::on_start_riding(m);
                        m.extra.mount = Some(id);
                        m.extra.passenger = true;
                        m.ai.nav.stop();
                        count += 1;
                        self.seat_mob(mob);
                        // Mob.startRiding drops its lead.
                        self.break_leash(mob, true, false);
                        self.note_started_riding(id);
                        self.vehicle_picked_up();
                    }
                    continue;
                }
                self.boat_push_mob(i, j, count);
            }
            if pushes {
                self.boat_push_mob(i, j, count);
            }
        }
        for (pb, own, touched, pushes) in players {
            let Some(v) = push_impulse(self.boats[i].position, pb.center())
                .filter(|_| boat_pushes(bb, pb, false))
            else {
                continue;
            };
            for _ in 0..usize::from(touched) + usize::from(pushes) {
                if count == 0 {
                    self.boats
                        .update(i, |b| b.velocity = b.velocity.subtract(v));
                }
                if own && let Some(p) = self.player.as_mut() {
                    p.velocity = p.velocity.add_vector(v);
                }
            }
        }
        for k in 0..self.boats.len() {
            let ob = self.boats[k].bounds();
            if k == i || !ob.intersects(area) || !boat_pushes(bb, ob, true) {
                continue;
            }
            let Some(v) = push_impulse(self.boats[i].position, self.boats[k].position) else {
                continue;
            };
            if count == 0 {
                self.boats
                    .update(i, |b| b.velocity = b.velocity.subtract(v));
            }
            if self.boat_passengers(self.boats[k].id) == 0 {
                self.boats
                    .update(k, |b| b.velocity = b.velocity.add_vector(v));
            }
        }
        let guest_carts = self.guest_vehicles();
        for c in 0..self.carts.len() {
            let cb = self.carts[c].bounds();
            if !cb.intersects(area) || !boat_pushes(bb, cb, false) {
                continue;
            }
            let Some(v) = push_impulse(self.boats[i].position, self.carts[c].position) else {
                continue;
            };
            if count == 0 {
                self.boats
                    .update(i, |b| b.velocity = b.velocity.subtract(v));
            }
            let cid = self.carts[c].id;
            let ridden = self.riding == Some(cid)
                || guest_carts.contains(&cid)
                || self.mobs.iter().any(|m| m.extra.mount == Some(cid));
            if !ridden {
                self.carts
                    .update(c, |c| c.velocity = c.velocity.add_vector(v));
            }
        }
    }
    /// `AbstractBoat.push(mob)` from boat `i`, which has `count` passengers.
    fn boat_push_mob(&mut self, i: usize, j: usize, count: usize) {
        let (b, m) = (&self.boats[i], &self.mobs[j]);
        if !boat_pushes(b.bounds(), m.bounds(), false) {
            return;
        }
        let Some(v) = push_impulse(b.position, m.position) else {
            return;
        };
        if count == 0 {
            self.boats
                .update(i, |b| b.velocity = b.velocity.subtract(v));
        }
        let m = &mut self.mobs[j];
        if moved_by_push(m) {
            m.velocity = m.velocity.add_vector(v);
            m.needs_sync = true;
        }
    }

    /// `positionRider` and `clampRotation` for the mobs in boats: each sits
    /// in its seat facing the way the boat does, an animal in a full boat
    /// sideways, its head kept within its reach of its body.
    pub(super) fn seat_boat_mobs(&mut self, riding: &super::passengers::Riders) {
        for i in 0..self.boats.len() {
            let id = self.boats[i].id;
            // `boat_mobs`, from the riders found in one pass.
            let mobs: Vec<usize> = riding
                .get(&id)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&j| !self.mobs[j].removed && self.mobs[j].alive())
                .collect();
            if mobs.is_empty() {
                continue;
            }
            let players = self.boat_players(id);
            let retained_seats =
                (!self.persist.off_level.is_empty()).then(|| self.seated_passengers(id));
            let count = retained_seats
                .as_ref()
                .map_or(players + mobs.len(), Vec::len);
            let b = &self.boats[i];
            let full = count == b.max_passengers();
            let delta = b.delta_rotation;
            let boat_box = b.bounds();
            let seats: Vec<_> = mobs
                .iter()
                .enumerate()
                .map(|(n, &j)| {
                    let m = &self.mobs[j];
                    let animal = is_animal(m.species());
                    // Another player's too, on a server's client.
                    // AbstractBoat's box is 1.375 wide.
                    let offset = riders::vehicle_attachment(m, 1.375);
                    let index = retained_seats
                        .as_ref()
                        .and_then(|seats| seats.iter().position(|&(_, id, _)| id == m.id))
                        .unwrap_or(players + n);
                    // calculatePassengerBodyYRot.
                    let turn = match (animal && full, m.id.is_multiple_of(2)) {
                        (false, _) => 0.0,
                        (true, true) => 90.0,
                        (true, false) => 270.0,
                    };
                    let turns = !self
                        .res
                        .mob_data()
                        .entity_has("minecraft:can_turn_in_boats", &m.kind.id);
                    (
                        j,
                        offset,
                        b.seat(index, count, animal, offset, false),
                        b.seat(index, count, animal, offset, true),
                        turns.then_some((b.yaw + turn, b.previous_yaw + turn)),
                    )
                })
                .collect();
            for (j, offset, at, previous, facing) in seats {
                let m = &mut self.mobs[j];
                m.seated(id, offset, boat_box);
                m.position = at;
                m.previous = previous;
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
                let Some((body, previous_body)) = facing else {
                    continue;
                };
                m.head_yaw += delta;
                m.body_yaw = body;
                m.previous_body_yaw = previous_body;
                m.yaw = body;
                m.previous_yaw = previous_body;
                // Mob.clampHeadRotationToBody.
                let limit = m.kind.max_head_y_rot;
                let d = wrap_degrees(body - m.head_yaw);
                m.head_yaw += d - d.clamp(-limit, limit);
            }
        }
    }
    /// `Entity.setRemoved` for a killed boat or cart; false on a reentrant
    /// removal of the same object while its dismount callbacks are running.
    pub(super) fn kill_vehicle_object(&mut self, target: super::commands::Target) -> bool {
        use super::{commands::Target, persist::RemovedVehicle};
        let id = match target {
            Target::Boat(id) | Target::Cart(id) => id,
            _ => unreachable!("only boat and cart removals use this helper"),
        };
        if !self.persist.notified_vehicle_removals.remove(&id)
            && self
                .with_destroying_vehicle(target, |game| {
                    game.dismount_destroyed_vehicle_players(id);
                    match target {
                        Target::Boat(_) => game.dismount_killed_boat(id),
                        Target::Cart(_) => game.dismount_killed_cart(id),
                        _ => unreachable!(),
                    }
                })
                .is_none()
        {
            return false;
        }
        match target {
            Target::Boat(_) => {
                if let Some(i) = self.boats.iter().position(|b| b.id == id) {
                    self.release_vehicle_container(id);
                    let boat = self.boats.remove(i);
                    self.drop_vehicle_contents(boat.position.components(), &boat.items);
                    self.boat_removed(&boat);
                    self.retain_removed_vehicle(id, RemovedVehicle::Boat(boat));
                }
            }
            Target::Cart(_) => {
                self.release_vehicle_container(id);
                if let Some(i) = self.carts.iter().position(|c| c.id == id) {
                    let cart = self.carts.remove(i);
                    self.drop_vehicle_contents(cart.position.components(), &cart.items);
                    self.retain_removed_vehicle(id, RemovedVehicle::Cart(cart));
                }
            }
            _ => unreachable!(),
        }
        true
    }

    /// `Entity.setRemoved` also attempts the player edges, using their own
    /// server context and rechecking each after earlier callbacks.
    pub(super) fn dismount_destroyed_vehicle_players(&mut self, vehicle: u32) {
        let players: Vec<_> = self
            .seated_passengers(vehicle)
            .into_iter()
            .filter_map(|(_, id, player)| player.then_some(id))
            .collect();
        let local_listed = self
            .lan_acting_entity
            .is_some_and(|id| players.contains(&id));
        for id in players {
            if self
                .seated_passengers(vehicle)
                .iter()
                .any(|&(_, rider, player)| player && rider == id)
                && let Some(rider) = self.entity(id)
            {
                self.entity_stop_riding(rider);
            }
        }
        // A local player outside a server guest turn has no seated entity ID.
        if !local_listed
            && self.lan_acting.is_none()
            && self.player.is_some()
            && self.riding == Some(vehicle)
        {
            self.stop_riding();
        }
    }

    /// `Entity.setRemoved`: attempt each passenger once; a veto keeps the edge.
    pub(super) fn dismount_destroyed_vehicle_mobs(&mut self, vehicle: u32, y: f64) {
        for rider in self.mob_passenger_ids(vehicle) {
            if self
                .mob_object(rider)
                .is_some_and(|mob| !mob.removed && mob.extra.mount == Some(vehicle))
            {
                self.dismount_cart_mob_at(rider, y);
            }
        }
    }

    /// `ejectPassengers` for the mobs in boat `i`: each gets out where
    /// `getDismountLocationForPassenger` puts it (beside the boat when it
    /// broke: `LivingEntity.dismountVehicle`) and can't board for a while.
    pub(super) fn eject_boat_mobs(&mut self, i: usize, broken: bool) {
        let b = &self.boats[i];
        let (id, y) = (b.id, b.position.y);
        for rider in self.mob_passenger_ids(id) {
            if self
                .mob_object(rider)
                .is_none_or(|m| m.removed || m.extra.mount != Some(id))
            {
                continue;
            }
            if !self.native_mob_may_ride(rider, id, false) {
                continue;
            }
            let Some(m) = self
                .mob_object(rider)
                .filter(|m| !m.removed && m.extra.mount == Some(id))
            else {
                continue;
            };
            let at = if broken {
                self.removed_vehicle_dismount_point(
                    m.position,
                    f64::from(m.width()),
                    f64::from(m.height()),
                    y,
                )
            } else if let Some(boat) = self.boats.iter().find(|b| b.id == id) {
                self.boat_dismount_target(boat, f64::from(m.width()), m.yaw)
            } else {
                self.removed_vehicle_dismount_point(
                    m.position,
                    f64::from(m.width()),
                    f64::from(m.height()),
                    y,
                )
            };
            let m = self.mob_object_mut(rider).unwrap();
            m.extra.mount = None;
            m.extra.passenger = false;
            m.extra.boarding_cooldown = BOARDING_COOLDOWN;
            m.position = at;
            m.previous = at;
            self.snap_mob_passengers(rider);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn rejected_boat_passengers_fill_actual_capacity_without_becoming_resident() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.boats.push(super::super::boats::Boat::new(
                71,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            for id in [72, 73] {
                let mut rider = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                rider.extra.mount = Some(71);
                game.persist
                    .off_level
                    .insert(id, super::super::persist::OffLevel::Mob(rider));
            }
            assert!(game.boat_mobs(71).is_empty());
            // An ordinary oak boat has two seats; rejected passengers still own both.
            assert_eq!(game.boat_passengers(71), 2);
            assert!(!game.boat_has_room(0));
            game.mob_object_mut(73).unwrap().extra.mount = None;
            assert_eq!(game.boat_passengers(71), 1);
            assert!(game.boat_has_room(0));
        });
    }

    #[test]
    fn push_is_capped_and_scaled() {
        // One block apart on x: the full 0.05 along x.
        let v = push_impulse(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap();
        assert!((v.x - f64::from(0.05_f32)).abs() < 1e-12 && v.z == 0.0);
        // Almost on the same spot: no push.
        assert!(push_impulse(Vec3::ZERO, Vec3::new(0.005, 0.0, 0.0)).is_none());
    }

    #[test]
    fn warden_and_wither_never_board() {
        assert!(!species_can_ride(Species::Warden));
        assert!(!species_can_ride(Species::Wither));
        assert!(species_can_ride(Species::Cow));
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_growing_a_mob_during_vehicle_ticks_keeps_it_in_collision_queries() {
        use super::super::native_entity_refs::tests::with_game;

        struct Resize;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Resize {}

        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            let mut cow = Mob::new(77, kind, Vec3::ZERO, 0.0);
            horizon_client::mobs::finalize::set_baby(&mut cow, true);
            game.mobs.push(cow);
            let mob = game.mob(game.entity(77).unwrap()).unwrap();
            let area = Aabb::new(Vec3::new(0.3, 0.0, 0.0), Vec3::new(0.4, 1.0, 0.1));
            assert!(!game.mobs[0].bounds().intersects(area));
            let mods = vec![mod_api::ModEntry::new("resize-test", Box::new(Resize))].into();
            mod_api::testing::with_mods(mods, || {
                game.vehicles_ticking(true);
                assert!(mob.set_baby(game, false));
                let cow = &game.mobs[0];
                assert!(cow.bounds().intersects(area));
                assert!(
                    game.vehicle_mob_reach()
                        .is_none_or(|reach| reach.may_meet(cow, area))
                );
                game.vehicles_ticking(false);
            });
        });
    }

    #[test]
    fn boats_push_by_height() {
        let boat = Aabb::new(Vec3::new(0.0, 64.0, 0.0), Vec3::new(1.0, 64.5625, 1.0));
        let at = |y: f64| Aabb::new(Vec3::new(0.0, y, 0.0), Vec3::new(1.0, y + 1.0, 1.0));
        assert!(boat_pushes(boat, at(64.0), false));
        assert!(!boat_pushes(boat, at(64.1), false));
        assert!(boat_pushes(boat, at(64.5), true));
        assert!(!boat_pushes(boat, at(64.5625), true));
    }
}

#[cfg(test)]
mod off_level_tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn retained_boat_passenger_occupies_a_seat_without_being_positioned() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let boat = Boat::new(71, "oak".into(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            let mut ghost = horizon_client::mobs::Mob::new(72, pig.clone(), Vec3::ZERO, 0.0);
            ghost.extra.mount = Some(71);
            ghost.extra.vehicle_order = 0;
            let mut resident = horizon_client::mobs::Mob::new(73, pig, Vec3::ZERO, 0.0);
            resident.extra.mount = Some(71);
            resident.extra.vehicle_order = 1;
            let offset = riders::vehicle_attachment(&resident, 1.375);
            let expected = boat.seat(1, 2, true, offset, false);
            game.boats.push(boat);
            game.mobs.push(resident);
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(ghost));
            game.position_riders();
            assert_eq!(game.mob_object(73).unwrap().position, expected);
            assert_eq!(game.mob_object(72).unwrap().position, Vec3::ZERO);
        });
    }
}

#[cfg(test)]
mod boat_callback_review_tests {
    use super::*;
    use horizon_client::mobs::Mob;
    use std::sync::{Arc, Mutex};

    struct MutateRider {
        discard: bool,
        heard: Arc<Mutex<Vec<(u32, bool)>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for MutateRider {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            self.heard
                .lock()
                .unwrap()
                .push((event.entity, event.mounting));
            if !event.mounting && event.entity == 72 {
                assert_eq!(
                    host.entity(72).unwrap().vehicle(host).map(|v| v.id()),
                    Some(71),
                    "callback observes original riding relationship"
                );
                if self.discard {
                    assert!(host.entity(72).unwrap().discard(host));
                } else {
                    let later = host.entity(73).unwrap();
                    let replacement = host.entity(74).unwrap();
                    assert!(later.start_riding(host, replacement, true));
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn boat_ejection_preserves_a_later_passengers_callback_reparenting() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: false,
                heard: heard.clone(),
            }),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.boats.push(super::super::boats::Boat::new(
                    71,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for id in [72, 73, 74] {
                    let mut mob = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                    mob.extra.mount = (id != 74).then_some(71);
                    mob.extra.vehicle_order = i64::from(id);
                    game.mobs.push(mob);
                }
                game.eject_boat_mobs(0, true);
                assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                assert_eq!(
                    heard
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|&&(id, mounting)| id == 73 && !mounting)
                        .count(),
                    1,
                    "only the callback's deliberate reparenting dismounts this rider"
                );
            });
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn boat_ejection_does_not_snap_a_rider_discarded_by_its_callback() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: true,
                heard: Arc::default(),
            }),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.boats.push(super::super::boats::Boat::new(
                    71,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let original = Vec3::new(2.0, 60.0, 3.0);
                let mut rider = Mob::new(72, kind.clone(), original, 0.0);
                rider.extra.mount = Some(71);
                game.mobs.push(rider);
                let mut child = Mob::new(73, kind, original, 0.0);
                child.extra.mount = Some(72);
                game.mobs.push(child);
                let mut boat = game.boats[0].clone();
                boat.position = Vec3::new(40.0, 70.0, 40.0);
                game.boats.store_back(0, boat);
                game.eject_boat_mobs(0, true);
                assert!(game.mob_object(72).unwrap().removed);
                assert_eq!(game.mob_object(72).unwrap().position, original);
                assert_eq!(game.mob_object(73).unwrap().position, original);
            });
        });
    }
}
