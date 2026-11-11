pub use horizon_core::random::LegacyRandom;
pub use horizon_core::{
    game_mode::GameMode, geometry::Direction, position::BlockPos, random::RandomSource,
};

/// Native engine Java Math paths. Results can differ by machine as vanilla's do.
/// On wasm use strict_math; java_math calls are unavailable there.
pub mod java_math;
/// Platform-independent StrictMath functions with established exact bits.
pub mod strict_math;
pub use horizon_core::nbt::{Error as NbtError, NETWORK_NBT_QUOTA};
