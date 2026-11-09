use super::*;
pub(crate) type Capabilities<P> = HashMap<&'static str, HashMap<Box<str>, Vec<CapabilityPart<P>>>>;

pub(crate) enum ProviderTarget<'a> {
    BlockEntity(&'a crate::BlockEntityTypeKey),
    Id(&'a str),
}

/// A capability provider a mod registered: a [`BlockProviderOf`], [`ItemProviderOf`] or
/// [`EntityProviderOf`].
#[derive(Clone)]
pub(crate) struct CapabilityPart<P> {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// The handler type it gives, for refusing a capability name answered with two.
    pub(crate) type_name: Arc<str>,
    /// Asked ahead of the game's own provider.
    pub(crate) before_game: bool,
    pub(crate) provider: P,
}

/// A capability provider whose handler type is a mod's own (a key a library's `[api]` crate
/// publishes), erased: only mods linked with that crate know its handler type.
// hidden: the registrar's erased provider, not part of the mod surface
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: transfers the handler name and a retained provider resource id, reporting unsupported handlers rather than serializing the native Any object.
#[doc(hidden)]
#[derive(Clone)]
pub struct OwnProvider {
    /// The `Arc` of the provider trait object, which a lookup of that handler type downcasts:
    /// a wasm mod's, in the game or another group, is the game's proxy, which no lookup takes.
    pub(crate) provider: Arc<dyn Any + Send + Sync>,
    pub(crate) handler: Arc<str>,
}

/// The registrar's table, as the game reads it.
pub(crate) struct InstalledProviders;

pub(crate) fn next<P: Clone>(
    table: &Capabilities<P>,
    name: &str,
    id: &str,
    index: &mut u32,
) -> Option<(P, &'static str, bool)> {
    next_provider(table.get(name)?.get(id)?, index)
}

/// A provider by its handler type: one case for each handler type `mod_api`'s keys name
/// (generated from them), and [`OwnProvider`] for a mod's own.
#[allow(missing_docs)]
pub(crate) mod providers_of {
    use super::*;
    // The keys' handler types and contexts, as the generated cases name them.
    #[allow(unused_imports)]
    use crate::*;
    include!(concat!(
        env!("OUT_DIR"),
        "/capability_providers_generated.rs"
    ));
}

/// Who answers a capability: a mod's provider, the game's own, or nobody.
pub(crate) enum Answer<T: ?Sized> {
    Mod(Capability<T>),
    Game,
    Nobody,
}

/// The handler `answer` names: a mod's, or `game`, the game's own when it answered.
pub(crate) fn answered<T: ?Sized>(
    answer: Answer<T>,
    game: Option<Box<T>>,
) -> Option<Capability<T>> {
    match answer {
        Answer::Mod(found) => Some(found),
        Answer::Game => game.map(|handler| Capability {
            handler,
            owner: None,
        }),
        Answer::Nobody => None,
    }
}

/// Asks the providers `next` gives (advancing a registration cursor in dispatch order: those
/// registered `before_game`, then the others), and the game's own (`game`: whether it answers) between the
/// two, in the fork's chosen order (NeoForge's javadoc doesn't state one between mods'
/// providers and the game's); the first to answer wins. `get` takes a provider of this
/// capability's handler type out of its case: a provider of a mod's own type the asker isn't
/// linked with is passed over.
pub(crate) fn ask_providers<O, P: ?Sized + 'static, T: ?Sized>(
    host: &mut dyn ServerHost,
    next: impl Fn(&mut u32) -> Option<(O, &'static str, bool)>,
    get: impl Fn(&O) -> Option<&Arc<P>>,
    call: impl Fn(&P, &mut dyn ServerHost) -> Option<Box<T>>,
    game: impl FnOnce(&mut dyn ServerHost) -> bool,
) -> Answer<T> {
    let mut game = Some(game);
    let mut index = 0;
    while let Some((provider, owner, before_game)) = next(&mut index) {
        if !before_game
            && let Some(game) = game.take()
            && game(host)
        {
            return Answer::Game;
        }
        if let Some(handler) = get(&provider).and_then(|provider| call(provider, host)) {
            return Answer::Mod(Capability {
                handler,
                owner: Some(owner),
            });
        }
    }
    match game.is_some_and(|game| game(host)) {
        true => Answer::Game,
        false => Answer::Nobody,
    }
}

impl PartRegistrar {
    /// Adds `provider` for capability `name` and `id` to `table`. Refused for a capability name
    /// or an id with no namespace, and for a capability name another provider answers with
    /// another handler type (two mods each declaring their own key under one name, rather than
    /// sharing the one the defining mod's `[api]` crate publishes): asked by one type, the other's
    /// providers would never answer.
    pub(crate) fn capability<P>(
        &mut self,
        table: fn(&mut Parts) -> &mut Capabilities<P>,
        name: &'static str,
        target: ProviderTarget<'_>,
        before_game: bool,
        (provider, handler): (P, impl Into<Arc<str>>),
    ) {
        let handler = handler.into();
        let me = self.mod_id;
        if !namespaced(name) {
            self.refused.push(format!(
                "{me} answered capability {name:?}, which is not a namespaced name"
            ));
            return;
        }
        let block_entity_id;
        let id = match target {
            ProviderTarget::BlockEntity(kind) => {
                block_entity_id = self
                    .registry_keys
                    .borrow()
                    .block_entity_types
                    .id(kind.0)
                    .map(str::to_owned);
                let Some(id) = block_entity_id.as_deref() else {
                    return;
                };
                id
            }
            ProviderTarget::Id(id) => {
                if !namespaced(id) {
                    self.refused.push(format!(
                        "{me} answered {name} for {id:?}, which is not a namespaced id"
                    ));
                    return;
                }
                id
            }
        };
        let by_id = table(&mut self.parts).entry(name).or_default();
        if let Some(other) = by_id.values().flatten().next()
            && other.type_name != handler
        {
            self.refused.push(format!(
                "capability {name} is answered with two handler types, {} by {} and {handler} by {me}",
                other.type_name, other.mod_id,
            ));
            return;
        }
        let providers = by_id.entry(id.into()).or_default();
        // Those asked ahead of the game first, each kind in registration order.
        let at = if before_game {
            providers.partition_point(|part| part.before_game)
        } else {
            providers.len()
        };
        providers.insert(
            at,
            CapabilityPart {
                mod_id: me,
                mod_index: self.mod_index,
                type_name: handler,
                before_game,
                provider,
            },
        );
    }
}
/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::testing::{FakeContainer, FakeHost};

    #[cfg(test)]
    /// `test:ordered` block entities from x = 7 to 10, where the game has slots of its own
    /// from x = 8 on.
    pub(crate) fn ordered_world() -> FakeHost {
        let mut host = FakeHost::new();
        for x in 7..=10 {
            let ordered = host.level.block_entity([x, 64, 0]).of_type("test:ordered");
            if x >= 8 {
                ordered.with_items(27);
            }
        }
        host
    }

    #[cfg(test)]
    /// A pipe at (0, 64, 0), and east of it a vanilla chest of one slot that holds one item,
    /// answered by the game.
    pub(crate) fn pipe_world() -> FakeHost {
        let mut host = FakeHost::new();
        host.level.block_entity(PIPE).of_type("test:pipe");
        host.level.block_entity(CHEST).items = Some(FakeContainer {
            slots: vec![None],
            limit: 1,
        });
        host
    }

    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::parts::tests::*;
    use crate::*;

    /// A chemical, as Mekanism's: the test mods' own resource, which the game knows nothing of.
    #[derive(Clone, Debug, PartialEq)]
    // exhaustive: the game builds it with a literal; mods only read it
    // no wire: testing-feature fixture state, not a mod boundary value.
    pub struct Chemical(pub String);

    /// The test mods' own capability, as Mekanism's `Capabilities.CHEMICAL`: the game has no
    /// answer for it, so only their providers do.
    pub const CHEMICALS: CapabilityKey<dyn ResourceHandler<Chemical>> =
        CapabilityKey::new("test:chemical");

    /// A mod block entity's one tank of 1000 of a chemical, kept in its saved data (`chemical`
    /// and `amount`), whose writes the game's journal keeps for a transaction.
    // exhaustive: the game builds it with a literal; mods only read it
    // no wire: testing-feature fixture state, not a mod boundary value.
    pub struct ChemTank(pub BlockSide);
    impl ChemTank {
        fn entity(&self, host: &dyn Host) -> Option<BlockEntityRef> {
            host.block_entity(self.0.level, self.0.pos)
        }
        fn held(&self, host: &dyn Host) -> Option<(Chemical, u32)> {
            let data = self.entity(host)?.data(host)?;
            let Some(Tag::String(chemical)) = data.get(&"chemical".into()) else {
                return None;
            };
            let Some(&Tag::Int(amount)) = data.get(&"amount".into()) else {
                return None;
            };
            let amount = u32::try_from(amount).ok().filter(|&a| a > 0)?;
            Some((Chemical(chemical.to_string_lossy()), amount))
        }
        fn set(&self, host: &mut dyn ServerHost, chemical: &Chemical, amount: u32) {
            let Some(entity) = self.entity(host) else {
                return;
            };
            let mut data = Compound::new();
            data.insert("chemical".into(), Tag::String(chemical.0.as_str().into()));
            data.insert("amount".into(), Tag::Int(amount as i32));
            let _ = entity.merge_data(host, data);
        }
    }
    impl ResourceHandler<Chemical> for ChemTank {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<Chemical> {
            self.held(host).map(|(chemical, _)| chemical)
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            self.held(host).map_or(0, |(_, amount)| amount)
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &Chemical) -> u32 {
            1000
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> u32 {
            let held = match self.held(host) {
                Some((other, _)) if other != *chemical => return 0,
                Some((_, held)) => held,
                None => 0,
            };
            let went = amount.min(1000 - held);
            if went > 0 {
                self.set(host, chemical, held + went);
            }
            went
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> u32 {
            let Some((held_chemical, held)) = self.held(host).filter(|(c, _)| c == chemical) else {
                return 0;
            };
            let came = amount.min(held);
            self.set(host, &held_chemical, held - came);
            came
        }
    }

    pub(crate) const ROBOT_ENERGY: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "energy",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    /// A fluid attachment type for the tests.
    pub const ROBOT_FLUID: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "fluid",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    /// A second test mod, sharing `CHEMICALS` and `Chemical` with the first, as a mod depending
    /// on another's `[api]` crate does: its vats answer the same capability.
    pub(crate) struct Chemist;
    #[mod_api::__subscriptions]
    impl Hooks for Chemist {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let chem_vat = r.block_entity_type_key("test:chem_vat");
            r.block_entity_capability(&CHEMICALS, &chem_vat, |_, at| Some(Box::new(ChemTank(at))));
        }
    }

    pub(crate) const CAPABILITY_TYPES: &[&str] = &[
        "pipe",
        "greedy",
        "item_overstates",
        "ordered",
        "tank",
        "gives_more",
        "takes_more",
        "battery_gives_more",
        "battery_takes_more",
        "chem_tank",
        "chem_vat",
    ];

    #[test]
    pub(crate) fn a_level_not_in_place_asks_no_provider() {
        game_thread();
        let mut world = pipe_world();
        let host: &mut dyn ServerHost = &mut world;
        // The pipe's type has a provider, which answers in the level in place; parked while
        // the server ticks another, the same block entity's level gives none.
        let parked = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::NETHER);
        assert!(here().capability(host, &ITEMS, PIPE, None).is_some());
        assert!(parked.capability(host, &ITEMS, PIPE, None).is_none());
    }

    #[test]
    pub(crate) fn a_mod_reads_a_vanilla_chests_items_from_the_game_with_no_provider_for_it() {
        game_thread();
        let mut world = pipe_world();
        let chest = world.level.block_entity(CHEST).items.as_mut().unwrap();
        chest.slots[0] = Some(stick());
        let host: &mut dyn ServerHost = &mut world;
        // No mod registered an items provider for the chest's type: the game's hoppers have no
        // mod's handler to look for there, yet a mod reading it gets the game's own slots.
        assert!(block_entity_providers(&ITEMS, host, on(CHEST, None)).is_none());
        let chest = here().capability(host, &ITEMS, CHEST, None).unwrap();
        assert_eq!(chest.owner(), "the game");
        assert_eq!(chest.size(host), 1);
        assert_eq!(chest.resource(host, 0), Some(ItemResource::of(&stick())));
    }

    #[test]
    pub(crate) fn a_capability_part_answers_by_side_and_forwards_to_its_neighbours_capability() {
        game_thread();
        let mut world = pipe_world();
        let host: &mut dyn ServerHost = &mut world;
        let up = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::Up))
            .unwrap();
        assert_eq!(up.size(host), 0);
        let pipe = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::North))
            .expect("the pipe's part");
        assert_eq!(pipe.size(host), 1);
        let stick_ = ItemResource::of(&stick());
        // The pipe calls the chest's capability, the game's own, with the host it was given:
        // in an aborted transaction the stick goes in, and comes back out.
        let aborted: Result<(), u32> =
            host.transaction(|host, tx| Err(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(aborted, Err(1));
        assert!(
            backend_of(&*host)
                .and_then(|backend| backend.container_stack(on(CHEST, None), 0))
                .is_none()
        );
        let went = host.transaction(|host, tx| Ok::<_, ()>(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(went, Ok(1));
        assert_eq!(
            backend_of(&*host).and_then(|backend| backend.container_stack(on(CHEST, None), 0)),
            Some(stick())
        );
        let went = host.transaction(|host, tx| Ok::<_, ()>(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(went, Ok(0));
        // No block entity with slots: no answer. A mod's own capability has no game answer.
        assert!(here().capability(host, &ITEMS, [5, 64, 0], None).is_none());
        const CHEMICALS: CapabilityKey<dyn ResourceHandler<ItemResource>> =
            CapabilityKey::new("test:chemicals");
        assert!(here().capability(host, &CHEMICALS, CHEST, None).is_none());
    }

    #[cfg(test)]
    /// Puts `amount` of `item` into slot 0 of [`ITEM_ITEMS`]' handler for the item at `place`, in a
    /// transaction committed when all went in: how much did.
    pub(crate) fn fill(
        host: &mut dyn ServerHost,
        place: &ItemPlace,
        item: ItemStack,
        amount: u32,
    ) -> u32 {
        let handler = place
            .capability(host, &ITEM_ITEMS)
            .expect("the item's part");
        let resource = ItemResource::of(&item);
        let went =
            host.transaction(
                |host, tx| match handler.insert(host, tx, 0, &resource, amount) {
                    went if went == amount => Ok(went),
                    went => Err(went),
                },
            );
        went.unwrap_or_else(|went| went)
    }

    /// A mod declaring its own key under a name another mod's providers answer with another
    /// handler type is refused at load, naming both mods and both types, as is a key whose name
    /// has no namespace; the same name with the same type, from another mod, is not.
    #[test]
    pub(crate) fn a_capability_name_answered_with_two_handler_types_is_refused() {
        const NOT_CHEMICALS: CapabilityKey<dyn ResourceHandler<FluidResource>> =
            CapabilityKey::new("test:chemical");
        const UNNAMESPACED: CapabilityKey<dyn ResourceHandler<Chemical>> =
            CapabilityKey::new("chemical");
        let mut r = super::PartRegistrar::new("first");
        let chem_tank = r.block_entity_type_key("test:chem_tank");
        (&mut r as &mut dyn Registrar)
            .block_entity_capability(&CHEMICALS, &chem_tank, |_, at| Some(Box::new(ChemTank(at))));
        r.mod_id = "second";
        r.mod_index = 1;
        let chem_vat = r.block_entity_type_key("test:chem_vat");
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &NOT_CHEMICALS,
            &chem_vat,
            |_, at| Some(Box::new(ModTank { at, lie: None })),
        );
        (&mut r as &mut dyn Registrar)
            .block_entity_capability(&CHEMICALS, &chem_vat, |_, at| Some(Box::new(ChemTank(at))));
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &UNNAMESPACED,
            &chem_vat,
            |_, at| Some(Box::new(ChemTank(at))),
        );
        assert_eq!(r.refused.len(), 2, "{:?}", r.refused);
        let two_types = &r.refused[0];
        assert!(
            two_types.starts_with("capability test:chemical is answered with two handler types")
        );
        let chemicals = type_name::<dyn ResourceHandler<Chemical>>();
        let fluids = type_name::<dyn ResourceHandler<FluidResource>>();
        assert!(
            two_types.contains(&format!("{chemicals} by first")),
            "{two_types}"
        );
        assert!(
            two_types.contains(&format!("{fluids} by second")),
            "{two_types}"
        );
        assert!(r.refused[1].contains("capability \"chemical\", which is not a namespaced name"));
    }

    /// A handler that only tells which provider made it, by its slot count.
    pub(crate) struct Marked(pub(crate) usize);
    impl ResourceHandler<ItemResource> for Marked {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            self.0
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    #[test]
    pub(crate) fn disabled_capability_providers_are_skipped_and_resume() {
        struct Later;
        #[mod_api::__subscriptions]
        impl Hooks for Later {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(99))));
                r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
                    Some(Box::new(Marked(99)))
                });
                r.entity_capability(&ENTITY_ITEMS, "test:robot", |_, _, ()| {
                    Some(Box::new(Marked(99)))
                });
            }
        }
        let mods: crate::Mods = vec![
            ModEntry::new("doubler", Box::new(Doubler)),
            ModEntry::new("later", Box::new(Later)),
        ]
        .into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();

        // These lookup objects keep their indexed registrations, not a snapshot of enabled mods.
        let at = BlockSide {
            level: here(),
            pos: [7, 64, 0],
            side: None,
        };
        let direct_block = || block_entity_providers(&ITEMS, &ordered_world(), at).unwrap();
        let robot = EntityRef::new(WorldEpoch::new(1), 3);
        let mut player = player_world(None);
        let direct_entity =
            || entity_providers(&ENTITY_ITEMS, &player_world(None), robot, ()).unwrap();
        let block_before_disable = direct_block();
        let entity_before_disable = direct_entity();
        let check = |without_game: Option<usize>, with_game: usize, robot_size: Option<usize>| {
            let host: &mut dyn ServerHost = &mut ordered_world();
            let block = here().capability(host, &ITEMS, [7, 64, 0], None);
            assert_eq!(block.map(|found| found.size(host)), without_game);
            let block = here().capability(host, &ITEMS, [9, 64, 0], None).unwrap();
            assert_eq!(block.size(host), with_game);
            assert_eq!(
                direct_block()
                    .ask(host, false)
                    .map(|found| found.size(host)),
                without_game
            );
            assert_eq!(
                direct_block().ask(host, true).map(|found| found.size(host)),
                None
            );
            for (damage, expected) in [(0, without_game), (1, Some(with_game))] {
                let host: &mut dyn ServerHost =
                    &mut HeldItem(stack("test:ordered_item", 1, damage));
                let item = HAND.capability(host, &ITEM_ITEMS);
                assert_eq!(item.map(|found| found.size(host)), expected);
            }
            let host: &mut dyn ServerHost = &mut player_world(None);
            let entity = robot.capability(host, &ENTITY_ITEMS, ());
            assert_eq!(entity.map(|found| found.size(host)), robot_size);
            assert_eq!(
                direct_entity()
                    .ask(host, false)
                    .map(|found| found.size(host)),
                robot_size
            );
        };
        // At x = 9 Doubler replaces the game's block provider, while the item with damage 1
        // uses the game's handler (27), so check that special block answer separately.
        assert_eq!(
            here()
                .capability(&mut ordered_world(), &ITEMS, [9, 64, 0], None)
                .unwrap()
                .size(&mut ordered_world()),
            3
        );
        assert!(crate::set_mod_enabled("doubler", false));
        check(Some(99), 27, Some(99));
        assert_eq!(
            block_before_disable
                .ask(&mut ordered_world(), false)
                .unwrap()
                .size(&mut ordered_world()),
            99
        );
        let entity = entity_before_disable.ask(&mut player, false).unwrap();
        assert_eq!(entity.size(&mut player), 99);
        assert!(crate::set_mod_enabled("later", false));
        check(None, 27, None);
        assert!(crate::set_mod_enabled("later", true));
        check(Some(99), 27, Some(99));
        assert!(crate::set_mod_enabled("doubler", true));
        assert_eq!(
            here()
                .capability(&mut ordered_world(), &ITEMS, [7, 64, 0], None)
                .unwrap()
                .size(&mut ordered_world()),
            1
        );
        assert_eq!(
            robot
                .capability(&mut player, &ENTITY_ITEMS, ())
                .unwrap()
                .size(&mut player),
            3
        );
        let host: &mut dyn ServerHost = &mut HeldItem(stack("test:ordered_item", 1, 0));
        assert_eq!(HAND.capability(host, &ITEM_ITEMS).unwrap().size(host), 1);
    }

    #[test]
    pub(crate) fn disabling_a_provider_during_lookup_does_not_skip_the_next_mod() {
        struct SwitchOff;
        #[mod_api::__subscriptions]
        impl Hooks for SwitchOff {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _| {
                    assert!(crate::set_mod_enabled("switch", false));
                    None
                });
                r.block_entity_capability(&ITEMS, &ordered, |_, _| {
                    panic!("a disabled mod's later provider must not run")
                });
            }
        }
        struct Later;
        #[mod_api::__subscriptions]
        impl Hooks for Later {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(99))));
            }
        }
        let mods: crate::Mods = vec![
            ModEntry::new("switch", Box::new(SwitchOff)),
            ModEntry::new("later", Box::new(Later)),
        ]
        .into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
        let found = here()
            .capability(&mut ordered_world(), &ITEMS, [7, 64, 0], None)
            .unwrap();
        assert_eq!(
            (found.owner(), found.size(&mut ordered_world())),
            ("later".into(), 99)
        );
    }

    #[test]
    pub(crate) fn providers_are_asked_ahead_of_the_game_then_the_game_then_the_mods_in_order() {
        game_thread();
        let host: &mut dyn ServerHost = &mut ordered_world();
        let answer = |host: &mut dyn ServerHost, x| {
            let found = here().capability(host, &ITEMS, [x, 64, 0], None).unwrap();
            (found.owner.is_some(), found.size(host))
        };
        // No game slots: the first mod provider that answers; the one before it declined.
        assert_eq!(answer(host, 7), (true, 1));
        // The game's slots come before the mods' providers.
        assert_eq!(answer(host, 8), (false, 27));
        // Providers registered before the game come first where they answer, in registration
        // order: at x = 9 both answer, at x = 10 only the second.
        assert_eq!(answer(host, 9), (true, 3));
        assert_eq!(answer(host, 10), (true, 4));
        // The game's own lookup takes a mod's handler only where the game has none or a mod
        // comes first.
        let mod_only = |host: &mut dyn ServerHost, x| {
            let at = BlockSide {
                level: here(),
                pos: [x, 64, 0],
                side: None,
            };
            let game_slots = backend_of(&*host)
                .and_then(|backend| backend.container_size(at))
                .is_some();
            let providers = block_entity_providers(&ITEMS, host, at)?;
            providers
                .ask(host, game_slots)
                .map(|found| found.size(host))
        };
        assert_eq!(mod_only(host, 7), Some(1));
        assert_eq!(mod_only(host, 8), None);
        assert_eq!(mod_only(host, 9), Some(3));
        // With no provider for the block entity's type, or no type (a vanilla one), there are
        // no providers to ask, so the game needn't work out its own answer: a hopper's lookup
        // costs one table check.
        let at = BlockSide {
            level: here(),
            pos: [8, 64, 0],
            side: None,
        };
        let mut vanilla = FakeHost::new();
        vanilla.level.block_entity(at.pos);
        assert!(block_entity_providers(&ITEMS, &vanilla, at).is_none());
        let mut unregistered = FakeHost::new();
        unregistered
            .level
            .block_entity(at.pos)
            .of_type("test:unregistered");
        assert!(block_entity_providers(&ITEMS, &unregistered, at).is_none());

        // Item providers in the same order, for the item in a player's hand: with no game slots
        // (damage 0), the first mod's that answers; with them (damage 1), the game's; for two
        // items, the one ahead of the game.
        let item = |damage, count| {
            let mut host = HeldItem(stack("test:ordered_item", count, damage));
            let host: &mut dyn ServerHost = &mut host;
            let found = HAND.capability(host, &ITEM_ITEMS).unwrap();
            (found.owner.is_some(), found.size(host))
        };
        assert_eq!(item(0, 1), (true, 1));
        assert_eq!(item(1, 1), (false, 27));
        assert_eq!(item(1, 2), (true, 3));
    }

    #[test]
    pub(crate) fn with_no_parts_the_block_entity_is_never_named() {
        let parts = Parts::default();
        let named = std::cell::Cell::new(false);
        let id = || {
            named.set(true);
            Some("test:press")
        };
        assert!(parts.block_entity(id).is_empty());
        assert!(!named.get(), "an empty table resolved the block's name");
    }
}

/// CapabilityProviders operations for capabilities.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait CapabilitiesCapabilityProviders {
    /// Whether any mod registered capability `name` for the things `by` names. This presence
    /// check includes disabled mods; enumeration skips their providers.
    fn provided(&self, by: ProvidedBy, name: &str) -> bool;
}

impl CapabilitiesCapabilityProviders for InstalledProviders {
    fn provided(&self, by: ProvidedBy, name: &str) -> bool {
        installed().is_some_and(|installed| match by {
            ProvidedBy::BlockEntity => installed.keys.capabilities.contains_key(name),
            ProvidedBy::Item => installed.keys.item_capabilities.contains_key(name),
            ProvidedBy::Entity => installed.keys.entity_capabilities.contains_key(name),
        })
    }
}
