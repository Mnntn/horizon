//! Chunk generators a native mod registers: vanilla's `ChunkGenerator` with
//! its own terrain, as
//! [`register_chunk_generator`](crate::Registrar::register_chunk_generator)
//! hands it over once at startup. A level whose stem names `modid:void`, one
//! [`register_dimension`](crate::Registrar::register_dimension) made or a data
//! pack's, then generates by the mod's code.
//!
//! There is no `Host` here: chunks fill on the worldgen worker threads, so the
//! mod sees only the [`ChunkAccess`] of the chunk it fills, and a generator
//! keeps no mutable state because two workers can fill neighbouring chunks at
//! once. The level gets terrain only: no surface, carvers or features.
use crate::{BlockKey, BlockMetadata, BlockState, ChunkPos};
use std::sync::Arc;

/// The chunk a generator fills: vanilla's `ChunkAccess`, as far as terrain needs it. Positions
/// are world blocks.
pub trait ChunkAccess {
    /// Immutable block metadata from this worker's resource snapshot.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// `ChunkAccess.getPos`.
    fn pos(&self) -> ChunkPos;
    /// The world's seed.
    fn seed(&self) -> i64;
    /// `getMinY`: the level's lowest block Y.
    fn min_y(&self) -> i32;
    /// `getHeight`: the level's build height, from its dimension type.
    fn height(&self) -> i32;
    /// The block's default state. Resolve before the terrain-writing loop.
    fn state(&self, block: BlockKey) -> BlockState;
    /// A block state with partial properties; missing properties keep defaults.
    /// Resolve before the terrain-writing loop, never once per placed block.
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState;
    /// `setBlockState`: write `state` at `pos`, answering whether it was written. A position
    /// outside the chunk or the build height is skipped.
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool;
}

/// A mod's chunk generator: vanilla's `ChunkGenerator`, whose code answers for
/// a level stem with `"generator": {"type": "modid:void"}`. Implemented once,
/// kept by the game; deterministic and stateless.
pub trait ChunkGenerator: Send + Sync + 'static {
    /// `ChunkGenerator.buildTerrain`: write the terrain of `chunk`, which
    /// starts as air. The same seed and chunk give the same blocks.
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess);
}

/// A registered API generator, before the game builds its worker adapter.
pub(crate) struct ChunkGeneratorInfo {
    pub(crate) id: Box<str>,
    pub(crate) generator: Arc<dyn ChunkGenerator>,
}

/// Registered generators for the hidden game integration boundary.
// hidden: the game's startup adapter reads the registered generators; mods declare them through the registrar.
#[doc(hidden)]
pub fn chunk_generator_declarations() -> Vec<(String, Arc<dyn ChunkGenerator>)> {
    crate::parts::installed_chunk_generator_list()
        .iter()
        .map(|info| (info.id.to_string(), info.generator.clone()))
        .collect()
}
