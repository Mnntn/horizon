//! Native terrain adapter for the API's worker context.
use super::native_worldgen::WorkerResources;
use horizon_world::worldgen::mod_generators::{
    ChunkAccess as EngineChunk, ModChunkGenerator, ModChunkGenerators,
};
use mod_api::world::structure_type::ChunkPos;
use mod_api::{BlockKey, BlockMetadata, BlockState, ChunkAccess, ChunkGenerator};
use std::sync::Arc;

struct Adapter(Arc<dyn ChunkGenerator>);
impl ModChunkGenerator for Adapter {
    fn build_terrain(&self, chunk: &mut dyn EngineChunk) {
        let Ok(worker) = WorkerResources::get(chunk.registry()) else {
            log::error!("Horizon: mod terrain has no resolved worker metadata");
            return;
        };
        self.0.build_terrain(&mut Converting {
            inner: chunk,
            worker,
        });
    }
}

struct Converting<'a> {
    inner: &'a mut dyn EngineChunk,
    worker: &'a WorkerResources,
}
impl ChunkAccess for Converting<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn pos(&self) -> ChunkPos {
        self.inner.pos()
    }
    fn seed(&self) -> i64 {
        self.inner.seed()
    }
    fn min_y(&self) -> i32 {
        self.inner.min_y()
    }
    fn height(&self) -> i32 {
        self.inner.height()
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker
            .state_with(self.inner.registry(), block, properties)
    }
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool {
        self.worker
            .index
            .to_engine(state)
            .is_ok_and(|state| self.inner.set_block(pos, state))
    }
}

pub(in crate::game) fn installed_chunk_generators() -> ModChunkGenerators {
    mod_api::__private::chunk_generator_declarations()
        .into_iter()
        .map(|(id, generator)| {
            (
                id,
                Arc::new(Adapter(generator)) as Arc<dyn ModChunkGenerator>,
            )
        })
        .collect()
}
