# Observing refused damage attempts

`entity_invulnerability_check` observes an attempted server hit without changing its amount.
It supplies the entity, damage source, original invulnerability decision and a writable decision.
A listener that only plays a sound leaves the decision unchanged. There is no amount: vanilla's
`isInvulnerableTo` takes the level and damage source, not damage. Amount mutation belongs on
`living_incoming_damage`, which runs after the early living guards but before the hurt-cooldown
comparison.

For player hits, the observation is delivered once across the server-player, player and living
predicates and the paused equipment continuations. The base decision is retained for that attempt;
the surrounding predicates still apply their own PvP, ability, gamerule, death and client-loaded
guards. A new hit, including one refused by cooldown, starts a new observation. Creative and
PvP-refused player wrappers do not suppress it. Ordinary mob hits also deliver it before death,
fire resistance, shield and cooldown refusal. Ghast and warden overrides deliver their complete
invulnerability decision, including flagged ghasts, reflected fireballs and digging wardens.

Zero-damage snowball, egg, ender-pearl and custom thrown collisions still enter the player
predicate before the zero-damage refusal. A successfully teleported player's pearl also attempts
its separate fall damage when a listener changes that damage to zero. Wither-skull impact and
its subsequent explosion are separate attempts. `/damage` asks the predicate even for spectators.

Firework damage to the attached entity is a direct attempt, including creative, spectator or dead
targets. Its splash query excludes spectators, but includes creative, dead and not-yet-loaded
resident players and dead resident mobs. Ability, death and client-loaded refusals occur inside
the damage pipeline, after the observation.

A damage-generating precondition is not itself an invulnerability query. In particular:

- Poison at health at or below one does not call vanilla `hurtServer` and produces no observation.
- Spectators are absent from vanilla's explosion entity query; team damage restrictions can also
  prevent an explosion damage call. Neither generates an observation. Creative players remain in
  the query and their attempted explosion damage is observed before ability refusal.
- Falling-block damage selects only living, alive, non-creative, non-spectator entities. Evoker
  fangs select living, alive entities. Pufferfish contact comes from the player's alive,
  non-spectator contact selection. Targets omitted by those vanilla queries have no attempt.
- A sulfur cube with a body item can consume body-immune damage in its `hurtServer` override,
  performing its special push and returning without entering the living invulnerability predicate.
  That path produces no observation, matching the absence of a vanilla predicate call. Ordinary
  sulfur cube damage that enters the living pipeline is observed.
- Client-side prediction, missing targets and unknown resident damage-source identifiers do not
  create authoritative server hit attempts.

The remaining selected non-living paths (armor stands, items, vehicles and experience orbs) retain
the coverage stated in the hook's reference documentation. A separate explicit invulnerability
query is a query, not an additional hit attempt.

The native regression test `native_hit_attempt_tests` counts actual listener notifications on
accepted and refused player and mob routes, equipment continuations, effects, explosions, and
small-fireball, wind-charge and shulker-bullet projectile entries. Unchanged listeners preserve the expected
health, refused fireballs restore prior fire, and only successful shulker hits grant levitation.
A decision-changing case checks that the modified answer survives continuations and a new attempt
asks again. The test uses prepared resources.

The additional native `remaining_hit_callers_observe_refusals_and_zero_damage` regression enters
actual skull, rocket, command and zero-damage projectile routes. It distinguishes attached from
splash spectator targets and checks resident dead mobs and dead or unloaded LAN guests. These
caller checks supplement the shared damage-pipeline tests rather than synthesizing hook calls.
