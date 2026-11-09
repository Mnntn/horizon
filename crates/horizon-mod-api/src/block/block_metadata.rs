//! The read-only metadata resource behind block value operations.
use crate::{BlockKey, StateConstraint};

/// Transport form for a process-local state. Mods use the BlockState facade, not this record.
/// Receiving metadata and write boundaries validate the ordinal against the block definition.
// hidden: generated native/wasm transport for the private-field BlockState facade.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockStateRecord {
    /// Process-local block key, not a saved block name.
    pub block: BlockKey,
    /// State ordinal within the block's definition.
    pub state: u16,
}
impl BlockStateRecord {
    /// Construct transport data; registry-dependent bounds belong to the receiving boundary.
    pub const fn new(block: BlockKey, state: u16) -> Self {
        Self { block, state }
    }
}

/// A refused metadata read or state operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BlockStateError {
    /// Metadata is unavailable in this context.
    Unavailable,
    /// The context does not contain the block key.
    UnknownBlock,
    /// The ordinal is outside the block definition.
    UnknownState,
    /// Constraint index naming a missing property.
    UnknownProperty(u32),
    /// Constraint index naming an invalid value.
    InvalidValue(u32),
    /// Constraint index repeating a property.
    DuplicateProperty(u32),
    /// The requested combination is not a registered state.
    InvalidCombination,
}
impl crate::NoBackend for BlockStateError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
impl From<horizon_mod_runtime::block_definitions::StateError> for BlockStateError {
    fn from(error: horizon_mod_runtime::block_definitions::StateError) -> Self {
        use horizon_mod_runtime::block_definitions::StateError;
        match error {
            StateError::UnknownState => Self::UnknownState,
            StateError::UnknownProperty(index) => Self::UnknownProperty(index as u32),
            StateError::InvalidValue(index) => Self::InvalidValue(index as u32),
            StateError::DuplicateProperty(index) => Self::DuplicateProperty(index as u32),
            StateError::InvalidCombination => Self::InvalidCombination,
        }
    }
}

/// Read-only metadata, lent by a host's block-states service or a tint worker.
/// This is the low-level resource contract; mods call the BlockState facade instead.
/// Native and generated WebAssembly forwards invoke the same immutable table implementation.
// hidden: low-level shared resource contract; BlockState methods are the mod-facing operations.
#[doc(hidden)]
pub trait BlockMetadata {
    /// Borrow the canonical block name from this context's retained registry.
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError>;
    /// Borrow a property's serialized value; a missing property returns None.
    fn property(
        &self,
        state: BlockStateRecord,
        name: &str,
    ) -> Result<Option<&str>, BlockStateError>;
    /// Change one property while retaining the others.
    fn with(
        &self,
        state: BlockStateRecord,
        name: &str,
        value: &str,
    ) -> Result<BlockStateRecord, BlockStateError>;
    /// Apply partial constraints to the block's default state.
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockStateRecord, BlockStateError>;
}

impl BlockMetadata for horizon_mod_runtime::block_definitions::Blocks {
    fn name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
        self.names
            .entries
            .get(block.0 as usize)
            .map(|entry| entry.id.as_str())
            .ok_or(BlockStateError::UnknownBlock)
    }
    fn property(
        &self,
        state: BlockStateRecord,
        name: &str,
    ) -> Result<Option<&str>, BlockStateError> {
        self.definitions
            .get(state.block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?
            .property(state.state, name)
            .map_err(Into::into)
    }
    fn with(
        &self,
        state: BlockStateRecord,
        name: &str,
        value: &str,
    ) -> Result<BlockStateRecord, BlockStateError> {
        let ordinal = self
            .definitions
            .get(state.block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?
            .with(state.state, name, value)?;
        Ok(BlockStateRecord::new(state.block, ordinal))
    }
    fn of(
        &self,
        block: BlockKey,
        properties: &[StateConstraint],
    ) -> Result<BlockStateRecord, BlockStateError> {
        let definition = self
            .definitions
            .get(block.0 as usize)
            .ok_or(BlockStateError::UnknownBlock)?;
        let constraints: Vec<_> = properties
            .iter()
            .map(|p| (p.name.as_str(), p.value.as_str()))
            .collect();
        let ordinal = definition.of(&constraints)?;
        Ok(BlockStateRecord::new(block, ordinal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, BackendRef, Host, HostBlockStates};
    struct MetadataHost(&'static horizon_mod_runtime::block_definitions::Blocks);
    impl Backend for MetadataHost {}
    crate::impl_backend_areas!(MetadataHost, except(block_states));
    impl crate::prelude::BlockStatesBackend for MetadataHost {
        fn block_metadata_name(&self, block: BlockKey) -> Result<&str, BlockStateError> {
            self.0.name(block)
        }
        fn block_metadata_property(
            &self,
            state: BlockStateRecord,
            name: &str,
        ) -> Result<Option<&str>, BlockStateError> {
            self.0.property(state, name)
        }
        fn block_metadata_with(
            &self,
            state: BlockStateRecord,
            name: &str,
            value: &str,
        ) -> Result<BlockStateRecord, BlockStateError> {
            self.0.with(state, name, value)
        }
        fn block_metadata_of(
            &self,
            block: BlockKey,
            properties: &[StateConstraint],
        ) -> Result<BlockStateRecord, BlockStateError> {
            self.0.of(block, properties)
        }
    }
    impl Host for MetadataHost {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    crate::impl_host_areas!(MetadataHost);
    struct Empty;
    impl Host for Empty {}
    crate::impl_host_areas!(Empty);

    #[test]
    fn metadata_service_reads_edits_and_refuses_invalid_transport() {
        crate::parts::tests::install_parts_fixture(&[]).unwrap();
        let states = [
            std::collections::BTreeMap::from([("lit".into(), "false".into())]),
            std::collections::BTreeMap::from([("lit".into(), "true".into())]),
        ];
        crate::__private::resolve_block_keys(["probe:lamp"], |_| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(
                &[&states[0], &states[1]],
                0,
            )
            .map_err(|e| format!("{e:?}"))
        })
        .unwrap();
        let host = MetadataHost(crate::__private::block_definitions().unwrap());
        let metadata = host.block_states();
        let block = crate::__private::block_key("probe:lamp").unwrap();
        assert_eq!(metadata.name(block), Ok("probe:lamp"));
        assert_eq!(
            metadata.name(BlockKey(999)),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            Empty.block_states().name(block),
            Err(BlockStateError::Unavailable)
        );
        let off = metadata.of(block, &[]).unwrap();
        assert_eq!(off.state, 0);
        let on = metadata.with(off, "lit", "true").unwrap();
        assert_eq!(on.state, 1);
        assert_eq!(metadata.property(on, "lit"), Ok(Some("true")));
        assert_eq!(metadata.property(off, "lit"), Ok(Some("false")));
        let borrowed = crate::__private::backend_read(&host, |backend| {
            backend.block_metadata_property(off, "lit")
        })
        .unwrap()
        .unwrap();
        assert_eq!(borrowed, Some("false"));
        let value = crate::BlockState::of(&metadata, block, &[]).unwrap();
        let changed = value.with(&metadata, "lit", "true").unwrap();
        assert_eq!(value.property(&metadata, "lit"), Ok(Some("false")));
        assert_eq!(changed.property(&metadata, "lit"), Ok(Some("true")));
        assert_eq!(changed.ordinal(), 1);
        let states = crate::__private::block_state_range(block).unwrap();
        assert_eq!(
            states
                .iter()
                .map(|state| state.property(&metadata, "lit").unwrap().unwrap())
                .collect::<Vec<_>>(),
            ["false", "true"]
        );
        assert_eq!(
            crate::__private::block_state_range(BlockKey(999)),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            changed.with(&metadata, "lit", "invalid"),
            Err(BlockStateError::InvalidValue(0))
        );
        assert_eq!(
            value.property(&Empty.block_states(), "lit"),
            Err(BlockStateError::Unavailable)
        );
        assert_eq!(metadata.property(off, "missing"), Ok(None));
        assert_eq!(
            metadata.with(off, "lit", "bad"),
            Err(BlockStateError::InvalidValue(0))
        );
        assert_eq!(
            metadata.property(BlockStateRecord::new(block, 2), "lit"),
            Err(BlockStateError::UnknownState)
        );
        assert_eq!(
            metadata.property(BlockStateRecord::new(BlockKey(999), 0), "lit"),
            Err(BlockStateError::UnknownBlock)
        );
        assert_eq!(
            Empty.block_states().property(off, "lit"),
            Err(BlockStateError::Unavailable)
        );
    }
}
