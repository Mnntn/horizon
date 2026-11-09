pub use world::dimension::{BiomeSourceKind, DimensionSettings, LevelDimensionType};
mod __private {
    /// The levels the mods registered, for the executable to hand to the
    /// engine before any world opens.
    pub use crate::world::dimension::dimension_declarations;
}
