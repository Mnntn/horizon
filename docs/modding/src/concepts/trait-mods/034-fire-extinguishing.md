<!-- chapter: Extinguishing fire -->

`EntityRef::extinguish_fire(host)` exposes vanilla `Entity.extinguishFire` on
players, guests, mobs, armor stands and dropped items. If the entity is burning,
it first emits `minecraft:entity.generic.extinguish_fire` at its feet with volume
0.7 and vanilla's pitch from two ordered random float draws. It then clears
positive remaining fire ticks, preserving a negative fire-immunity countdown.
Fire-immune mob kinds and dropped stacks resistant to in-fire damage do not
count as burning, even when their timer is positive.

The operation reuses the engine's mob sound outbox and entity random, and the
player/armor-stand cosmetic random and captured sound route. Dropped items use
their own random. Sounds use the entity's sound source, including `Players` for
a guest rather than a generic sound category. It does not draw pitch randomness
for a non-burning entity. It returns false for stale or unsupported handles.

`EntityRef::clear_fire(host)` is the distinct vanilla `Entity.clearFire` operation:
it is silent, draws no random, and preserves the same negative countdown. Choose
it only when the original behavior explicitly calls `clearFire`, rather than
using it as a substitute for extinguishing.
