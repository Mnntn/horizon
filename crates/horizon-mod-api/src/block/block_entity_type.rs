//! Typed identities and declarations of block entity types.
use crate::BlockEntityTypeKey;
use std::fmt;

/// A native Part cannot be attached to this type during registration.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockEntityPartError {
    /// Vanilla entities do not expose a native Part's payload.
    VanillaType {
        /// The block entity type that was refused.
        kind: BlockEntityTypeKey,
    },
    /// This Part type is already attached to the entity type.
    Duplicate {
        /// The block entity type that was refused.
        kind: BlockEntityTypeKey,
        /// The Part type that is already attached.
        part: &'static str,
    },
}

impl fmt::Display for BlockEntityPartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VanillaType { kind } => write!(
                f,
                "cannot attach a native block entity Part to vanilla type {}",
                crate::content::registry_keys::block_entity_type_display_name(*kind)
            ),
            Self::Duplicate { kind, part } => {
                write!(
                    f,
                    "Part {part} is attached to type {} twice",
                    crate::content::registry_keys::block_entity_type_display_name(*kind)
                )
            }
        }
    }
}

impl std::error::Error for BlockEntityPartError {}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// Builds one native block entity type during mod registration.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockEntityTypeBuilder<'a> {
    pub(crate) registrar: &'a mut dyn crate::Registrar,
    pub(crate) id: String,
    pub(crate) blocks: Vec<String>,
}

impl BlockEntityTypeBuilder<'_> {
    /// The blocks which may contain this type.
    pub fn blocks(mut self, blocks: &[&str]) -> Self {
        self.blocks = blocks.iter().map(|block| (*block).to_owned()).collect();
        self
    }

    /// Register the type and return its typed key.
    pub fn build(self) -> BlockEntityTypeKey {
        self.registrar
            .declare_block_entity_type(&self.id, self.blocks)
    }
}

impl crate::BlockEntityTypeAddBlocksEvent<'_> {
    /// Add blocks to one registered type, leaving its factory unchanged.
    pub fn modify(&mut self, kind: BlockEntityTypeKey, blocks: &[crate::BlockKey]) {
        self.additions
            .extend(blocks.iter().map(|&block| (kind, block)));
    }

    /// Add blocks to every registered type accepted by `predicate`.
    pub fn modify_matching(
        &mut self,
        mut predicate: impl FnMut(BlockEntityTypeKey) -> bool,
        blocks: &[crate::BlockKey],
    ) {
        for &kind in self.types {
            if predicate(kind) {
                self.additions
                    .extend(blocks.iter().map(|&block| (kind, block)));
            }
        }
    }
}
