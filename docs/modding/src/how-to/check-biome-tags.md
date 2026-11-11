# Check biome tags

Use the world's live biome tags without loading chunks:

```rust,ignore
use mod_api::{Host, LevelRef};

fn is_forest(host: &dyn Host, level: LevelRef, biome: &str) -> Option<bool> {
    host.level_biome_in_tag(level, biome, "minecraft:is_forest")
}
```

The tag may also start with `#`. An undefined tag returns `Some(false)` for a known biome. An unknown biome or unavailable level returns `None`; do not confuse that with a known biome outside the tag. Data-pack biome tags participate in this query.
