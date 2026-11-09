//! Explicit resource codecs for the API's erased handler traits.
//!
//! Handlers transfer through the current resource scope, never by copying their
//! concrete Rust state. Traits cannot derive Wire; this is their shared codec.
use crate::wire::{Reader, Wire, WireError, Writer};
use crate::*;
use std::sync::Arc;

macro_rules! arc_handler {
    ($handler:ty, $name:literal) => {
        impl Wire for Arc<$handler> {
            const LAYOUT_SEED: u64 = crate::wire::layout_name(concat!("Arc<dyn ", $name, ">"));
            const MIN: usize = 8;
            fn put(&self, out: &mut Writer) {
                out.put_resource(Box::new(self.clone()));
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                input.take_resource()
            }
        }
    };
}
macro_rules! handler {
    ($handler:ty, $name:literal) => {
        arc_handler!($handler, $name);
        impl Wire for Box<$handler> {
            const LAYOUT_SEED: u64 = crate::wire::layout_name(concat!("Box<dyn ", $name, ">"));
            const MIN: usize = 8;
            fn put(&self, out: &mut Writer) {
                out.fail(WireError::Invalid("boxed handler needs owned encoding"));
            }
            fn put_owned(self, out: &mut Writer) {
                out.put_resource(Box::new(Arc::<$handler>::from(self)));
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                input.take_resource()
            }
        }
    };
}
handler!(dyn Hooks, "Hooks");
handler!(dyn ReloadApply, "ReloadApply");
handler!(dyn StructureType, "StructureType");
handler!(dyn StructurePiece, "StructurePiece");
handler!(dyn BlockPart, "BlockPart");
handler!(dyn BlockEntityPart, "BlockEntityPart");
handler!(dyn ItemPart, "ItemPart");
handler!(dyn MenuPart, "MenuPart");
handler!(dyn FluidPart, "FluidPart");
handler!(dyn ServerReloadListener, "ServerReloadListener");
handler!(dyn GameTestFunction, "GameTestFunction");
handler!(dyn ClientReloadListener, "ClientReloadListener");
handler!(dyn BiomeColorResolver, "BiomeColorResolver");
handler!(dyn BlockTintSource, "BlockTintSource");
handler!(dyn ParticleProvider, "ParticleProvider");
handler!(dyn EnchantmentDamageEffect, "EnchantmentDamageEffect");
handler!(dyn CommandArgumentType, "CommandArgumentType");
handler!(dyn GroundStateProvider, "GroundStateProvider");
handler!(dyn PlaceProvider, "PlaceProvider");
handler!(dyn TooltipAppender, "TooltipAppender");
handler!(dyn CriterionTest, "CriterionTest");
handler!(
    dyn ResourceHandler<ItemResource>,
    "ResourceHandler<ItemResource>"
);
handler!(
    dyn ResourceHandler<FluidResource>,
    "ResourceHandler<FluidResource>"
);
handler!(dyn EnergyHandler, "EnergyHandler");
handler!(
    dyn BlockCapabilityProvider<dyn ResourceHandler<ItemResource>>,
    "BlockCapabilityProvider<dyn ResourceHandler<ItemResource>>"
);
handler!(
    dyn BlockCapabilityProvider<dyn ResourceHandler<FluidResource>>,
    "BlockCapabilityProvider<dyn ResourceHandler<FluidResource>>"
);
handler!(
    dyn BlockCapabilityProvider<dyn EnergyHandler>,
    "BlockCapabilityProvider<dyn EnergyHandler>"
);
handler!(
    dyn ItemCapabilityProvider<dyn ResourceHandler<ItemResource>>,
    "ItemCapabilityProvider<dyn ResourceHandler<ItemResource>>"
);
handler!(
    dyn ItemCapabilityProvider<dyn ResourceHandler<FluidResource>>,
    "ItemCapabilityProvider<dyn ResourceHandler<FluidResource>>"
);
handler!(
    dyn ItemCapabilityProvider<dyn EnergyHandler>,
    "ItemCapabilityProvider<dyn EnergyHandler>"
);
handler!(
    dyn EntityCapabilityProvider<dyn ResourceHandler<ItemResource>, ()>,
    "EntityCapabilityProvider<dyn ResourceHandler<ItemResource>, ()>"
);
handler!(
    dyn EntityCapabilityProvider<dyn ResourceHandler<ItemResource>, Option<Direction>>,
    "EntityCapabilityProvider<dyn ResourceHandler<ItemResource>, Option<Direction>>"
);
handler!(
    dyn EntityCapabilityProvider<dyn ResourceHandler<FluidResource>, Option<Direction>>,
    "EntityCapabilityProvider<dyn ResourceHandler<FluidResource>, Option<Direction>>"
);
handler!(
    dyn EntityCapabilityProvider<dyn EnergyHandler, Option<Direction>>,
    "EntityCapabilityProvider<dyn EnergyHandler, Option<Direction>>"
);
arc_handler!(dyn std::any::Any + Send + Sync, "Any");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Id, ResourceCodec, ResourceScope, Slots};
    use std::{
        any::{Any, TypeId},
        cell::RefCell,
        rc::Rc,
        sync::atomic::{AtomicU32, Ordering},
    };

    #[derive(Default)]
    struct Codec(RefCell<Slots<Box<dyn Any>>>);
    impl ResourceCodec for Codec {
        fn store(&self, value: Box<dyn Any>) -> Result<Id, WireError> {
            Ok(self.0.borrow_mut().insert(value))
        }
        fn load(&self, id: Id, _: TypeId) -> Result<Box<dyn Any>, WireError> {
            self.0
                .borrow_mut()
                .remove(id)
                .map_err(|_| WireError::Invalid("stale handler"))
        }
    }

    #[test]
    fn boxed_transfer_and_shared_transfer_preserve_callable_state() {
        let calls = Arc::new(AtomicU32::new(0));
        let state = calls.clone();
        let owned = <dyn ReloadApply>::new(move || {
            state.fetch_add(7, Ordering::SeqCst);
        });
        let _scope = ResourceScope::new(Rc::new(Codec::default()));
        let mut out = Writer::new();
        out.put(&owned);
        assert_eq!(
            out.error(),
            Some(WireError::Invalid("boxed handler needs owned encoding"))
        );
        assert!(out.bytes().is_empty());
        out.clear();
        out.put_owned(owned);
        assert_eq!(out.error(), None);
        assert_eq!(out.bytes(), &[0, 0, 0, 0, 1, 0, 0, 0]);
        let shared: Arc<dyn ReloadApply> = Reader::decode(out.bytes()).unwrap();
        out.clear();
        out.put(&shared);
        assert_eq!(out.error(), None);
        let restored: Arc<dyn ReloadApply> = Reader::decode(out.bytes()).unwrap();
        assert!(Arc::ptr_eq(&shared, &restored));
        restored.apply();
        assert_eq!(calls.load(Ordering::SeqCst), 7);
    }

    #[test]
    fn erased_handler_keeps_identity_and_refuses_expired_resource_scope() {
        let value: Arc<dyn Any + Send + Sync> = Arc::new(73u32);
        let mut out = Writer::new();
        {
            let _scope = ResourceScope::new(Rc::new(Codec::default()));
            out.put(&value);
            let restored: Arc<dyn Any + Send + Sync> = Reader::decode(out.bytes()).unwrap();
            assert!(Arc::ptr_eq(&value, &restored));
            assert_eq!(restored.downcast_ref::<u32>(), Some(&73));
        }
        assert!(matches!(
            Reader::decode::<Arc<dyn Any + Send + Sync>>(out.bytes()),
            Err(WireError::Invalid("no resource context"))
        ));
    }
}
