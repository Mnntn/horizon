<!-- chapter: Consumable effects -->
<!-- separator: 1 -->

A `LivingRef` on the server can apply one effect from an item's loaded consumable with
`living.apply_consume_effect(host, &stack, index)`. The index is zero-based in component
order; this does not eat food, shrink the stack or run consume triggers. `Some(true)` or
`Some(false)` is the effect's result; `None` means the item, index or target is unavailable
or the effect is unsupported. Mob random teleport, armor stands and dragons are not
supported. Player random teleport uses the level random as normal.

Mod-defined kinds dispatch through the ordinary `consume_effect` hook. For these
kinds, `Some(true)` means dispatch was entered, not that a listener changed game
state; an unclaimed kind still does nothing. Reapplying the same kind from inside
its own handler on the same call stack returns `Some(false)` without recursion.
Another kind, including one owned by another mod, dispatches normally, and a later
independent call can apply the original kind again. This deliberately reverses
#550's original blanket exclusion of mod-defined kinds; it is not a second consume
hook or an alternate dispatch path.
