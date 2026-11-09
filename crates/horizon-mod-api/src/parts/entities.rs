use super::*;
/// What answers an entity capability for one entity type: given the entity and the lookup's
/// context (its [`EntityCapabilityKey`]'s `C`), its handler, or none to leave it to the next
/// provider. A closure is one.
pub trait EntityCapabilityProvider<T: ?Sized, C>: Send + Sync {
    /// The handler for `entity`, asked with `context`, or none.
    fn provide(&self, host: &mut dyn ServerHost, entity: EntityRef, context: C) -> Option<Box<T>>;
}
impl<T: ?Sized, C, F> EntityCapabilityProvider<T, C> for F
where
    F: Fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>> + Send + Sync,
{
    fn provide(&self, host: &mut dyn ServerHost, entity: EntityRef, context: C) -> Option<Box<T>> {
        self(host, entity, context)
    }
}
// no wire: trait-object alias inherits its provider trait policy; callbacks are not serialized.
/// An entity capability provider, as the registrar keeps it.
pub type EntityProvider<T, C = ()> = dyn EntityCapabilityProvider<T, C>;

/// Whether any mod attached an entity part, so `Mob` construction asks for
/// goals and the mob tick builds a goal read view.
pub fn has_entity_parts() -> bool {
    !crate::mods().is_empty()
        && installed().is_some_and(|installed| !installed.keys.entities.is_empty())
}

/// The entity parts attached to entity type `id` (`minecraft:zombie`), in
/// dispatch order, for the goal registration at every `Mob` construction
/// (#1077).
pub fn entity_parts() -> PartsRef {
    let Some(installed) = installed() else {
        return PartsRef::Shared(&NO_PARTS);
    };
    if let Some(parts) = GAME_PARTS.with_borrow(Option::clone) {
        return PartsRef::Game(parts);
    }
    if !GAME_THREAD.get() {
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed");
        if !OFF_THREAD_WARNED.swap(true, Ordering::Relaxed) {
            log::warn!(
                "Native entity parts asked off a game thread ({thread}): a mob built there keeps no mod goals"
            );
        }
    }
    PartsRef::Shared(&installed.keys)
}

impl Parts {
    /// The entity parts attached to entity type `id` (`minecraft:zombie`), in
    /// dispatch order, for the goal registration at every `Mob` construction
    /// (#1077).
    pub fn entity_parts(&self, id: &str) -> &[Attached<dyn EntityPart>] {
        self.entities.get(id).map_or(&[], Vec::as_slice)
    }
}

/// Whether any mod registered a provider of entity capability `key`, for any entity type: one
/// table lookup, no allocation, so the game's own lookups skip all their work where none did.
pub fn has_entity_providers<T: ?Sized + 'static, C: 'static>(
    key: &EntityCapabilityKey<T, C>,
) -> bool {
    installed()
        .and_then(|installed| installed.keys.entity_capabilities.get(key.name))
        .is_some_and(|by_kind| !by_kind.is_empty())
}

/// The providers mods registered for `key` and the type of `entity`, for the game's own
/// lookups (a hopper's): none where no mod registered one, so a vanilla container entity's
/// lookup costs one table check and allocates nothing, and the game needn't work out its own
/// answer. [`EntityProviders::ask`] then asks them, as the game's lookups for block entities do.
pub fn entity_providers<T: ?Sized + 'static, C: Copy + 'static>(
    key: &EntityCapabilityKey<T, C>,
    host: &dyn Host,
    entity: EntityRef,
    context: C,
) -> Option<EntityProviders<T, C>> {
    let parts = installed()?
        .keys
        .entity_capabilities
        .get(key.name)?
        .get(&*entity.kind(host)?)?;
    Some(EntityProviders {
        parts,
        entity,
        context,
        handler: PhantomData,
    })
}

// no wire: provider lookup borrows static capability dispatch tables and process-local type identity.
/// An entity's providers of one capability ([`entity_providers`]).
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct EntityProviders<T: ?Sized + 'static, C: 'static> {
    pub(crate) parts: &'static [CapabilityPart<EntityProviderOf>],
    pub(crate) entity: EntityRef,
    pub(crate) context: C,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static, C: Copy + 'static> EntityProviders<T, C> {
    /// A mod's handler where the game would answer with its own unless a mod comes first: one
    /// registered `before_game`, else, where the game has no answer (`game`), the first mod's.
    pub fn ask(self, host: &mut dyn ServerHost, game: bool) -> Option<Capability<T>> {
        let (entity, context) = (self.entity, self.context);
        match ask_providers(
            host,
            |index| next_provider(self.parts, index),
            EntityProviderOf::get::<T, C>,
            |provider, host| provider.provide(host, entity, context),
            |_| game,
        ) {
            Answer::Mod(found) => Some(found),
            Answer::Game | Answer::Nobody => None,
        }
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to the entity type `entity` (`minecraft:zombie`): its
    /// [`EntityPart::register_goals`] runs at every `Mob` construction of that
    /// type, spawning or loading, and its goals join the mob's selectors
    /// beside vanilla's (#1077).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn entity_part<P: EntityPart>(&mut self, entity: &str, part: P) {
        self.attach_entity_part(entity, type_name::<P>(), Arc::new(part));
    }
    /// Answer entity capability `key` for entity type `kind` (`mymod:robot`,
    /// `minecraft:player`) with `provider`, which gives a handler for one entity, or none:
    /// `r.entity_capability(&ENTITY_ITEMS, "mymod:robot", |_, robot, ()| Some(Box::new(Cargo(robot))))`.
    /// Asked as `block_entity_capability`'s are.
    pub fn entity_capability<T: ?Sized + 'static, C: 'static>(
        &mut self,
        key: &EntityCapabilityKey<T, C>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_entity_capability(
            key.name,
            kind,
            false,
            EntityProviderOf::of(Arc::new(provider)),
        );
    }
    /// `entity_capability`, asked ahead of the game's own provider,
    /// as a mod that replaces how a player's inventory is reached.
    pub fn entity_capability_before_game<T: ?Sized + 'static, C: 'static>(
        &mut self,
        key: &EntityCapabilityKey<T, C>,
        kind: &str,
        provider: impl Fn(&mut dyn ServerHost, EntityRef, C) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_entity_capability(
            key.name,
            kind,
            true,
            EntityProviderOf::of(Arc::new(provider)),
        );
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use crate::parts::tests::*;
    use crate::*;

    #[test]
    pub(crate) fn entity_parts_refuse_unnamespaced_ids_and_doubles() {
        struct Once;
        #[mod_api::horizon_part]
        impl super::EntityPart for Once {
            fn register_goals(&self, goals: &mut crate::GoalRegistrar<'_>) {
                let _ = goals;
            }
        }
        struct Twice;
        #[mod_api::horizon_part]
        impl super::EntityPart for Twice {
            fn register_goals(&self, goals: &mut crate::GoalRegistrar<'_>) {
                let _ = goals;
            }
        }
        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                r.entity_part("zombie", Twice);
                r.entity_part("minecraft:zombie", Once);
                r.entity_part("minecraft:zombie", Once);
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].contains("to \"zombie\", which is not a namespaced id"),
            "{errors:?}"
        );
        assert!(
            errors[1].contains("attached to minecraft:zombie twice"),
            "{errors:?}"
        );
    }

    pub(crate) struct EntityBattery(pub(crate) EntityRef);
    impl EnergyHandler for EntityBattery {
        fn amount(&self, host: &mut dyn ServerHost) -> u32 {
            ROBOT_ENERGY
                .get_existing(host, Holder::Entity(self.0.id()))
                .unwrap()
                .unwrap_or(0)
        }
        fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
            1000
        }
        fn insert(&self, host: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            let moved = amount.min(1000 - held);
            ROBOT_ENERGY
                .set(host, Holder::Entity(self.0.id()), held + moved)
                .unwrap();
            moved
        }
        fn extract(&self, host: &mut dyn ServerHost, _: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            let moved = amount.min(held);
            ROBOT_ENERGY
                .set(host, Holder::Entity(self.0.id()), held - moved)
                .unwrap();
            moved
        }
    }

    pub(crate) struct EntityTank(pub(crate) EntityRef);
    impl ResourceHandler<FluidResource> for EntityTank {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<FluidResource> {
            (self.amount(host, 0) > 0).then(|| {
                FluidResource::of(&FluidStack::new(
                    crate::content::registry_keys::fluid_key("minecraft:water").unwrap(),
                    1,
                ))
            })
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            ROBOT_FLUID
                .get_existing(host, Holder::Entity(self.0.id()))
                .unwrap()
                .unwrap_or(0)
        }
        fn capacity(&self, _: &mut dyn ServerHost, index: usize, resource: &FluidResource) -> u32 {
            if index == 0
                && resource
                    == &FluidResource::of(&FluidStack::new(
                        crate::content::registry_keys::fluid_key("minecraft:water").unwrap(),
                        1,
                    ))
            {
                1000
            } else {
                0
            }
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> u32 {
            let held = self.amount(host, index);
            let moved = amount.min(self.capacity(host, index, resource).saturating_sub(held));
            ROBOT_FLUID
                .set(host, Holder::Entity(self.0.id()), held + moved)
                .unwrap();
            moved
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            index: usize,
            resource: &FluidResource,
            amount: u32,
        ) -> u32 {
            let held = self.amount(host, index);
            let moved = if self.capacity(host, index, resource) > 0 {
                amount.min(held)
            } else {
                0
            };
            ROBOT_FLUID
                .set(host, Holder::Entity(self.0.id()), held - moved)
                .unwrap();
            moved
        }
    }
}

/// Registrar operations for entities.
pub trait EntitiesRegistrar {
    /// Attach `part`, of kind `kind`, to the entity type `entity`
    /// (`minecraft:zombie`, `mymod:golem`): its `register_goals` runs at every
    /// `Mob` construction of that type (#1077).
    ///
    /// Not on wasm: the goal surface a part's `register_goals` hands the game
    /// does not cross yet (#1077); entity parts are a native mod's for now.
    /// The gate keeps a wasm mod's call a compile error, not a runtime one;
    /// the generator still keeps the method off the wasm world by this doc.
    #[cfg(not(target_arch = "wasm32"))]
    fn attach_entity_part(&mut self, entity: &str, kind: &'static str, part: Arc<dyn EntityPart>);

    /// Answer entity capability `name` for entity type `kind` with `provider`; mods reach it
    /// through `entity_capability`.
    fn provide_entity_capability(
        &mut self,
        name: &'static str,
        kind: &str,
        before_game: bool,
        provider: EntityProviderOf,
    );
}

impl EntitiesRegistrar for PartRegistrar {
    #[cfg(not(target_arch = "wasm32"))]
    fn attach_entity_part(&mut self, entity: &str, kind: &'static str, part: Arc<dyn EntityPart>) {
        self.attach(|p| &mut p.entities, kind, entity, part);
    }

    fn provide_entity_capability(
        &mut self,
        name: &'static str,
        kind: &str,
        before_game: bool,
        provider: EntityProviderOf,
    ) {
        self.capability(
            |p| &mut p.entity_capabilities,
            name,
            ProviderTarget::Id(kind),
            before_game,
            {
                let handler = provider.handler();
                (provider, handler)
            },
        );
    }
}

/// CapabilityProviders operations for entities.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait EntitiesCapabilityProviders {
    /// The next enabled provider of entity capability `name` for entity type `kind`, advancing `index`.
    fn entity_provider(
        &self,
        name: &str,
        kind: &str,
        index: &mut u32,
    ) -> Option<(EntityProviderOf, &'static str, bool)>;
}

impl EntitiesCapabilityProviders for InstalledProviders {
    fn entity_provider(
        &self,
        name: &str,
        kind: &str,
        index: &mut u32,
    ) -> Option<(EntityProviderOf, &'static str, bool)> {
        next(&installed()?.keys.entity_capabilities, name, kind, index)
    }
}
