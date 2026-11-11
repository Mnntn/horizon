<!-- chapter: A block's fluids -->
<!-- separator: 1 -->

#### A block's fluids

Fluids are the capability `FLUIDS` (NeoForge's `Capabilities.Fluid.BLOCK`), answered as items are, by a
`ResourceHandler<FluidResource>` in millibuckets. A `FluidResource` is a fluid id and its data components, without
an amount, kept as an item stack's are (a potion fluid's contents; water and lava have none), and a tank fills only
from empty or with the same fluid and the same components. A `FluidStack` is one with an amount, as tanks keep it
(`resource.to_stack(amount)`, `FluidResource::of(&stack)`). The game's own answer is a mod block's tanks by side
(`block_entity_fluids`) and a cauldron, one tank holding the content its block has: vanilla's water (a bucket in
thirds) and lava (a bucket only whole), or one a mod registered through `register_cauldron_fluid_content`, filled
and drained by whole levels; a cauldron has no block entity and is answered by position, and a powder snow
cauldron holds no fluid. A cauldron takes the fluids that have a content whatever their components and discards them, so
what comes out of it is plain: the one place a fluid's components are dropped. A mod block's saved tanks take no
fluid with components. `level.capability(host, &FLUIDS, pos, side)` finds a handler in a `LevelRef`'s level, as
`ITEMS`' are found, and `move_resources` moves fluids as it moves items. A move the target takes only part of is
undone and tried again with that part, so a cauldron, which gives whole thirds and never more than asked, gives
what fits: asked for a bucket with room for 500 mB, it gives one level, 334 mB.

Fluid held by an item is `ITEM_FLUIDS` (NeoForge's `Capabilities.Fluid.ITEM`), found where its stack lives, as
`ITEM_ITEMS` is: `place.capability(host, &ITEM_FLUIDS)` gives a `ResourceHandler<FluidResource>` for the item at
that `ItemPlace`, and a write rewrites the item there by an exchange of one, so from a stack of more than one the
changed item splits off (`r.item_capability(&ITEM_FLUIDS, "mymod:canister", …)` answers it for a mod's item). The
game answers it for vanilla's buckets: one tank of 1000 mB, filled and
drained whole, an empty bucket taking plain water or lava and becoming its full bucket, a full one becoming an empty
bucket. `move_resources` moves between an item and a block as between two blocks: a full cauldron fills a bucket in a
player's slot, and two thirds of one fill none.
