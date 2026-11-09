pub use world::structure_type::{
    BiomeId, ChunkPos, FoundStructure, GenerationContext, HeightmapType, NoiseBlock, NoiseColumn,
    PieceContext, StructureError, StructurePiece, StructureRandom, StructureType,
};
mod __private {
    /// The registered structure types, for the executable to hand to the engine's
    /// `StructureRegistry` before any world opens.
    pub use crate::world::structure_type::{biome_id, structure_type_declarations};
}
