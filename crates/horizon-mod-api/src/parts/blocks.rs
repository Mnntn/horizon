use super::*;
/// Whether this hook has a prepared subscriber for an API block key.
/// The caller owns validation of the routing key.
pub fn block_key_listens(hook: HookId, key: crate::BlockKey) -> bool {
    parts(hook, "block routing")
        .for_hook(hook)
        .is_some_and(|table| !table.block_key(Some(key)).is_empty())
}

impl HookParts {
    /// Prepared subscribers for an API block key; missing keys have no subscribers.
    pub fn block_key(&self, key: Option<crate::BlockKey>) -> &[Attached<dyn BlockPart>] {
        key.and_then(|key| self.block_keys.get(&key))
            .map_or(&[], |parts| parts.as_slice())
    }
    /// Parts for a checked API state. Key lookup does not depend on world state numbers.
    pub fn block_value(&self, state: crate::BlockState) -> &[Attached<dyn BlockPart>] {
        let Some(blocks) = crate::__private::block_definitions() else {
            return &[];
        };
        let Some(definition) = blocks.definitions.get(state.block().0 as usize) else {
            return &[];
        };
        if definition.property(state.ordinal(), "").is_err() {
            return &[];
        }
        self.block_key(Some(state.block()))
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to block `block` (`mymod:shaft`, `minecraft:oak_log`).
    pub fn block_part<P: BlockPart>(&mut self, block: crate::BlockKey, part: P) {
        self.attach_block_part(block, type_name::<P>(), Arc::new(part));
    }
    /// Attach `part` to every block in tag `tag` (`minecraft:logs`, without `#`), as each game's
    /// tags have it when its world opens and after `/reload`.
    pub fn block_part_tag<P: BlockPart>(&mut self, tag: &str, part: P) {
        self.attach_block_part_tag(tag, type_name::<P>(), Arc::new(part));
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use super::*;
    #[cfg(test)]
    use crate::parts::tests::*;
    use crate::*;

    /// A comparator reading of 5 more than it was.
    pub(crate) struct Gauge;
    #[mod_api::horizon_part]
    impl BlockPart for Gauge {
        #[cfg_attr(test, inline)]
        fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
            event.output += 5;
        }
        #[cfg_attr(test, cfg(any()))]
        fn should_check_weak_power(&self, _: &dyn LevelHost, _: &mut ShouldCheckWeakPowerEvent) {}
    }

    #[test]
    pub(crate) fn registry_keys_resolve_reused_parts_and_rebind_without_changing_identity() {
        use crate::content::registry_keys::block_id;
        struct Keys(Arc<std::sync::Mutex<Vec<BlockKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Keys {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let vanilla = r.block_key("stone");
                let same = r.block_key("minecraft:stone");
                let later = r.block_key("later:machine");
                assert_eq!(vanilla, same);
                assert_eq!(BlockKey::from_slot(vanilla.0), Ok(vanilla));
                assert_eq!(BlockKey::from_slot(later.0), Ok(later));
                assert!(BlockKey::from_slot(later.0 + 1).is_err());
                r.block_part(vanilla, Gauge);
                r.block_part(same, Connector);
                r.block_part(later, Gauge);
                *self.0.lock().unwrap() = vec![vanilla, same, later];
            }
        }
        let requested = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "requester",
            Box::new(Keys(requested.clone())),
        )]
        .into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
        let keys = requested.lock().unwrap().clone();
        assert_eq!(BlockKey::from_slot(keys[0].0), Ok(keys[0]));
        assert!(BlockKey::from_slot(2).is_err());
        // The final registry contains content declared after the requester's registration.
        let states = ["minecraft:stone", "later:machine", "later:machine"];
        crate::content::registry_keys::resolve_block_keys(
            ["minecraft:stone", "later:machine"],
            |_| {
                let properties = std::collections::BTreeMap::new();
                horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                    .map_err(|error| format!("{error:?}"))
            },
        )
        .unwrap();
        resolve_parts(
            &HashMap::new(),
            &HashMap::new(),
            KnownIds {
                block: &|id| states.contains(&id),
                item: &|_| false,
            },
        )
        .unwrap();
        let table = parts(HookId::AnalogOutput, "analog_output");
        let table = table.for_hook(HookId::AnalogOutput).unwrap();
        let value = crate::BlockState::from(crate::BlockStateRecord::new(keys[0], 0));
        assert_eq!(table.block_value(value).len(), 1);
        let invalid = crate::BlockState::from(crate::BlockStateRecord::new(keys[0], u16::MAX));
        assert!(table.block_value(invalid).is_empty());
        assert_eq!(block_id(keys[0]), Some("minecraft:stone"));
        assert_eq!(block_id(keys[2]), Some("later:machine"));
        assert!(BlockKey::from_slot(u32::MAX).is_err());
        assert_eq!(BlockKey::from_slot(keys[0].0), Ok(keys[0]));
        assert_eq!(block_id(keys[0]), Some("minecraft:stone"));
    }

    /// Connects redstone wire to its block from any side.
    pub(crate) struct Connector;
    #[mod_api::horizon_part]
    impl BlockPart for Connector {
        fn should_redstone_wire_connect_to(
            &self,
            _: &dyn LevelHost,
            event: &mut ShouldRedstoneWireConnectToEvent,
        ) {
            event.connected = true;
        }

        fn connects_to(&self, _: &dyn LevelHost, event: &mut ConnectsToEvent) {
            event.connected = event.kind == ConnectionKind::Wall;
        }
    }

    /// Makes a fence, wall or pane flip vanilla's answer toward the east and refuse every other
    /// side.
    pub(crate) struct ConnectionRule;
    #[mod_api::horizon_part]
    impl BlockPart for ConnectionRule {
        fn connects_to(&self, _: &dyn LevelHost, event: &mut ConnectsToEvent) {
            event.connected = event.direction == Direction::East && !event.connected;
        }
    }

    /// A comparator reading of 100 more, for the blocks in tag `test:gauged`.
    pub(crate) struct TagGauge;
    #[mod_api::horizon_part]
    impl BlockPart for TagGauge {
        fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
            event.output += 100;
        }
    }

    /// Install a signal part on glass for a real-game query.
    pub fn signal_game_thread() {
        struct Signal;
        #[mod_api::horizon_part]
        impl BlockPart for Signal {
            fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
                assert_eq!(
                    level.block_state([event.pos.x, event.pos.y, event.pos.z]),
                    Some(event.state)
                );
                event.signal = if event.direct { -2 } else { 20 };
            }
        }
        struct SignalMod;
        #[mod_api::__subscriptions]
        impl Hooks for SignalMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let block_key = r.block_key("minecraft:glass");
                r.block_part(block_key, Signal);
            }
        }
        let mods: crate::Mods = vec![ModEntry::new("signal_test", Box::new(SignalMod))].into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
    }

    /// Install a weak-power part on glass for a real-game query.
    pub fn weak_power_game_thread() {
        struct WeakPower;
        #[mod_api::horizon_part]
        impl BlockPart for WeakPower {
            fn should_check_weak_power(
                &self,
                _: &dyn LevelHost,
                event: &mut ShouldCheckWeakPowerEvent,
            ) {
                event.check = true;
            }
        }
        struct WeakPowerMod;
        #[mod_api::__subscriptions]
        impl Hooks for WeakPowerMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let block_key = r.block_key("minecraft:glass");
                r.block_part(block_key, WeakPower);
            }
        }
        let mods: crate::Mods =
            vec![ModEntry::new("weak_power_test", Box::new(WeakPowerMod))].into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
    }

    /// Install parts that change what a path may cross: glass becomes land and air, air
    /// stops being land and becomes water.
    pub fn pathfindable_game_thread() {
        struct Answers(&'static [(PathComputationType, bool)]);
        #[mod_api::horizon_part]
        impl BlockPart for Answers {
            fn is_pathfindable(&self, _: &dyn Host, event: &mut IsPathfindableEvent) {
                if let Some((_, answer)) = self.0.iter().find(|(kind, _)| *kind == event.kind) {
                    event.pathfindable = *answer;
                }
            }
        }
        struct PathMod;
        #[mod_api::__subscriptions]
        impl Hooks for PathMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let block_key = r.block_key("minecraft:glass");
                r.block_part(
                    block_key,
                    Answers(&[
                        (PathComputationType::Land, true),
                        (PathComputationType::Air, true),
                    ]),
                );
                let block_key = r.block_key("minecraft:air");
                r.block_part(
                    block_key,
                    Answers(&[
                        (PathComputationType::Land, false),
                        (PathComputationType::Water, true),
                    ]),
                );
            }
        }
        let mods: crate::Mods = vec![ModEntry::new("path_test", Box::new(PathMod))].into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
    }

    /// Install a part on glass that asks, when a glass block is removed, for every neighbour to
    /// be told, as an override of `affectNeighborsAfterRemoval` calling `updateNeighborsAt`.
    pub fn removal_game_thread() {
        struct Leaver;
        #[mod_api::horizon_part]
        impl BlockPart for Leaver {
            fn affect_neighbors_after_removal(
                &self,
                _: &dyn LevelHost,
                event: &mut AffectNeighborsAfterRemovalEvent<'_>,
            ) {
                event.updates.push(NeighbourUpdate::Neighbours);
            }
        }
        struct LeaverMod;
        #[mod_api::__subscriptions]
        impl Hooks for LeaverMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let block_key = r.block_key("minecraft:glass");
                r.block_part(block_key, Leaver);
            }
        }
        let mods: crate::Mods = vec![ModEntry::new("leaver_test", Box::new(LeaverMod))].into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
    }

    /// Install a connection rule on oak fences, cobblestone walls and glass panes for real-game
    /// tests: it flips vanilla's answer.
    pub fn connection_game_thread() {
        struct ConnectionMod;
        #[mod_api::__subscriptions]
        impl Hooks for ConnectionMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                for block in [
                    "minecraft:oak_fence",
                    "minecraft:cobblestone_wall",
                    "minecraft:glass_pane",
                ] {
                    let block_key = r.block_key(block);
                    r.block_part(block_key, ConnectionRule);
                }
            }
        }
        let mods: crate::Mods =
            vec![ModEntry::new("connection_test", Box::new(ConnectionMod))].into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
    }

    std::thread_local! {
        static SIGNAL_READS: std::cell::RefCell<Vec<SignalRead>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// What the signal-reading glass saw: in `neighbor_changed`, whether it is powered and
    /// how strongly, and whether its tick was scheduled; or that its tick ran.
    #[derive(Clone, Debug, PartialEq, Eq)]
    // exhaustive: a test fixture's record, matched whole by the game's test
    // no wire: testing-feature fixture state, not a mod boundary value.
    pub enum SignalRead {
        /// `neighbor_changed`: `has_neighbor_signal`, `best_neighbor_signal`, the scheduling's answer.
        Changed([i32; 3], bool, i32, bool),
        /// `tick_block` ran at this position, reading the signal from its south side.
        Ticked([i32; 3], i32),
    }

    /// The reads the signal-reading glass recorded on this thread, taken.
    pub fn signal_reads() -> Vec<SignalRead> {
        SIGNAL_READS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    /// Install a part on glass that, when a neighbour changes, reads its inputs and
    /// schedules its own tick 2 ticks later, as a diode does, for real-game tests.
    pub fn signal_reader_game_thread() {
        struct Reader(BlockKey);
        #[mod_api::horizon_part]
        impl BlockPart for Reader {
            fn neighbor_changed(
                &self,
                level: &mut dyn crate::LevelTicksHost,
                event: &mut crate::NeighborChangedEvent,
            ) {
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let read = SignalRead::Changed(
                    pos,
                    level.has_neighbor_signal(pos),
                    level.best_neighbor_signal(pos),
                    level.schedule_block_tick(pos, self.0, 2),
                );
                SIGNAL_READS.with(|r| r.borrow_mut().push(read));
            }
            fn tick_block(
                &self,
                level: &mut dyn crate::LevelWriteHost,
                event: &mut crate::TickBlockEvent,
            ) -> Flow {
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let south = [pos[0], pos[1], pos[2] + 1];
                let read = SignalRead::Ticked(pos, level.signal(south, Direction::South));
                SIGNAL_READS.with(|r| r.borrow_mut().push(read));
                Flow::Continue
            }
        }
        struct ReaderMod;
        #[mod_api::__subscriptions]
        impl Hooks for ReaderMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let block_key = r.block_key("minecraft:glass");
                r.block_part(block_key, Reader(block_key));
            }
        }
        let mods: crate::Mods =
            vec![ModEntry::new("signal_reader_test", Box::new(ReaderMod))].into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
    }

    /// Install a redstone connector on glass for real-game tests.
    pub fn redstone_game_thread() {
        struct RedstoneMod;
        #[mod_api::__subscriptions]
        impl Hooks for RedstoneMod {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let block_key = r.block_key("minecraft:glass");
                r.block_part(block_key, Connector);
            }
        }
        let mods: crate::Mods = vec![ModEntry::new("redstone_test", Box::new(RedstoneMod))].into();
        crate::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
    }

    #[cfg(test)]
    fn value(state: u32) -> BlockState {
        let name = [
            "minecraft:air",
            "test:gauge",
            "test:other",
            "test:connector",
            "test:press",
        ]
        .get(state as usize)
        .copied()
        .unwrap_or("test:gauge");
        let key = crate::content::registry_keys::block_key(name).unwrap();
        BlockState::from(BlockStateRecord::new(
            key,
            if state > 4 { u16::MAX } else { 0 },
        ))
    }

    #[cfg(test)]
    /// The comparator reading of state `state`, from 1.
    pub(crate) fn read(state: u32) -> i32 {
        let mut output = 1;
        dispatch_analog_output(
            &Level,
            LevelDimension::Overworld,
            BlockPos::new(0, 64, 0),
            value(state),
            Direction::North,
            &mut output,
        );
        output
    }

    #[cfg(test)]
    /// Resolve this thread's parts with tag `test:gauged` holding `members`.
    pub(crate) fn resolve(members: &[&str]) -> Result<(), super::ResolvePartsError> {
        crate::content::registry_keys::resolve_block_keys(
            [
                "minecraft:air",
                "minecraft:granite",
                "test:gauge",
                "test:other",
                "test:connector",
                "test:press",
            ],
            |_| {
                horizon_mod_runtime::block_definitions::BlockDefinition::new(
                    &[&Default::default()],
                    0,
                )
                .map_err(|e| format!("{e:?}"))
            },
        )
        .unwrap();
        let tags = HashMap::from([(
            "test:gauged".to_owned(),
            members.iter().map(|m| m.to_string()).collect(),
        )]);
        resolve_parts(
            &tags,
            &HashMap::new(),
            KnownIds {
                block: &|_| true,
                item: &|_| true,
            },
        )
    }

    #[test]
    fn prepared_key_routing_tracks_tag_reload() {
        game_thread();
        crate::content::registry_keys::resolve_block_keys(
            [
                "minecraft:air",
                "test:gauge",
                "test:other",
                "test:connector",
                "test:press",
                "minecraft:granite",
            ],
            |_| {
                horizon_mod_runtime::block_definitions::BlockDefinition::new(
                    &[&Default::default()],
                    0,
                )
                .map_err(|e| format!("{e:?}"))
            },
        )
        .unwrap();
        let other = crate::content::registry_keys::block_key("test:other").unwrap();
        resolve(&["test:other"]).unwrap();
        assert!(block_key_listens(HookId::AnalogOutput, other));
        resolve(&[]).unwrap();
        assert!(!block_key_listens(HookId::AnalogOutput, other));
        assert!(has_scoped_hook(HookId::AnalogOutput));
        assert!(resolve(&["missing:block"]).is_err());
        assert!(!block_key_listens(HookId::AnalogOutput, other));
        assert_eq!(
            read(1),
            6,
            "a refused preparation preserves the published Parts"
        );
    }

    #[test]
    pub(crate) fn tagged_dispatch_requires_its_own_world_table() {
        game_thread();
        resolve(&["test:gauge"]).unwrap();
        let installed_parts = super::TEST_PARTS.get();
        let failure = std::thread::Builder::new()
            .name("part-worker".into())
            .spawn(move || {
                super::TEST_PARTS.set(installed_parts);
                assert!(super::try_parts(HookId::ShouldCheckWeakPower, "other").is_ok());
                let error = super::try_parts(HookId::AnalogOutput, "analog_output")
                    .err()
                    .unwrap();
                let panic = std::panic::catch_unwind(|| read(1)).unwrap_err();
                (error, panic)
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(failure.0.hook, "analog_output");
        assert_eq!(failure.0.thread, "part-worker");
        let message = failure.1.downcast_ref::<String>().unwrap();
        assert!(message.contains("analog_output") && message.contains("part-worker"));
        assert_eq!(read(1), 106);
        super::clear_game_parts();
        assert_eq!(read(1), 1);
        resolve(&["test:gauge"]).unwrap();
        assert_eq!(read(1), 106);
        mark_game_thread();
        assert_eq!(read(1), 1);
    }

    #[test]
    pub(crate) fn a_block_part_answers_for_its_block_only() {
        game_thread();
        // No world is open, so no state has its parts yet: nothing answers.
        assert_eq!(read(1), 1);
        resolve(&[]).unwrap();
        // 5 more from the part: 1 + 5.
        assert_eq!(read(1), 6);
        assert_eq!(read(2), 1);
    }

    #[test]
    pub(crate) fn scoped_dispatch_looks_up_only_subscribed_hooks_and_parts() {
        game_thread();
        resolve(&[]).unwrap();
        let parts = super::parts(HookId::AnalogOutput, "test");
        let analog = parts.for_hook(HookId::AnalogOutput).unwrap();
        let connect = parts.for_hook(HookId::ShouldRedstoneWireConnectTo).unwrap();
        assert_eq!(analog.block_value(value(1)).len(), 1);
        assert!(connect.block_value(value(1)).is_empty());
        assert_eq!(connect.block_value(value(3)).len(), 1);
        assert!(analog.block_value(value(3)).is_empty());
        // The block entity part's BlockPart override is indexed independently.
        assert_eq!(read(4), 41);
        assert!(!super::has_part_hook(HookId::ShouldCheckWeakPower));
        super::TEST_PART_LOOKUPS.set(0);
        let mut check = false;
        dispatch_should_check_weak_power(
            &Level,
            LevelDimension::Overworld,
            BlockPos::new(0, 64, 0),
            value(1),
            Direction::East,
            &mut check,
        );
        assert_eq!(super::TEST_PART_LOOKUPS.get(), 0);
        assert!(!check);
    }

    #[test]
    pub(crate) fn redstone_connection_is_scoped_to_the_attached_block() {
        game_thread();
        resolve(&[]).unwrap();
        for (state, expected) in [(3, true), (2, false)] {
            let mut connected = false;
            dispatch_should_redstone_wire_connect_to(
                &Level,
                LevelDimension::Overworld,
                BlockPos::new(0, 64, 0),
                value(state),
                Some(Direction::East),
                &mut connected,
            );
            assert_eq!(connected, expected);
        }
    }

    #[test]
    pub(crate) fn a_connection_is_answered_only_by_the_blocks_own_part_with_its_kind() {
        game_thread();
        resolve(&[]).unwrap();
        for kind in [
            ConnectionKind::Fence,
            ConnectionKind::Wall,
            ConnectionKind::Bars,
        ] {
            // `test:connector` answers whether the kind is a wall, from the
            // opposite answer; `test:other` has no part and keeps it.
            let answer = kind == ConnectionKind::Wall;
            for (state, expected) in [(3, answer), (2, !answer)] {
                let mut connected = !answer;
                dispatch_connects_to(
                    &Level,
                    crate::__private::ConnectsToCall {
                        dimension: LevelDimension::Overworld,
                        pos: BlockPos::new(0, 64, 0),
                        state: value(state),
                        kind,
                        neighbour: value(1),
                        direction: Direction::West,
                        face_solid: true,
                        connected: &mut connected,
                    },
                );
                assert_eq!(connected, expected, "{kind:?} on state {state}");
            }
        }
    }

    #[test]
    pub(crate) fn a_state_without_parts_is_answered_by_nothing() {
        game_thread();
        resolve(&[]).unwrap();
        // Out of range, and in range but a block no part is attached to.
        assert!(
            super::parts(HookId::AnalogOutput, "analog_output")
                .for_hook(HookId::AnalogOutput)
                .unwrap()
                .block_value(value(99))
                .is_empty()
        );
        assert!(
            super::parts(HookId::AnalogOutput, "analog_output")
                .for_hook(HookId::AnalogOutput)
                .unwrap()
                .block_value(value(0))
                .is_empty()
        );
        // A scoped hook has no global method, so both keep vanilla's answer.
        assert_eq!(read(0), 1);
        assert_eq!(read(99), 1);
    }

    #[test]
    pub(crate) fn each_game_answers_from_its_own_tags_and_reload_drops_what_left() {
        let mods = game_thread();
        // Both games resolve before either reads, so one shared table would fail one of them.
        let resolved = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let client = scope.spawn(|| {
                crate::install(mods.clone());
                install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
                mark_game_thread();
                resolve(&["test:other"]).unwrap();
                resolved.wait();
                (read(1), read(2))
            });
            let server = scope.spawn(|| {
                crate::install(mods.clone());
                install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
                mark_game_thread();
                resolve(&["test:gauge"]).unwrap();
                resolved.wait();
                let before = (read(1), read(2));
                // `/reload` with the tag emptied.
                resolve(&[]).unwrap();
                (before, read(1))
            });
            // The tag part comes after the key part of the same mod: 1 + 5 + 100.
            assert_eq!(client.join().unwrap(), (6, 101));
            assert_eq!(server.join().unwrap(), ((106, 1), 6));
        });
    }

    #[test]
    pub(crate) fn a_key_the_game_lacks_fails_naming_the_mod() {
        game_thread();
        let refused = resolve_parts(
            &HashMap::new(),
            &HashMap::new(),
            KnownIds {
                block: &|id| id != "test:gauge",
                item: &|_| true,
            },
        )
        .unwrap_err();
        let super::ResolvePartsError::Parts(refused) = refused else {
            panic!("wrong refusal")
        };
        assert_eq!(refused.len(), 1, "{refused:?}");
        assert!(refused[0].starts_with("doubler attached "));
        assert!(refused[0].contains("Gauge to block test:gauge, which this game does not have"));
    }

    #[test]
    pub(crate) fn a_tag_giving_a_part_twice_keeps_the_earlier_mod() {
        let part = |mod_index| Attached::<dyn BlockPart> {
            mod_id: ["earlier", "later"][mod_index],
            mod_index,
            order: 0,
            type_name: "Gauge",
            part: Arc::new(Gauge),
        };
        let mut table = HashMap::from([("test:gauge".into(), vec![part(1)])]);
        let tags = HashMap::from([(
            "test:gauged".to_owned(),
            HashSet::from(["test:gauge".to_owned()]),
        )]);
        super::add_tagged(
            &mut table,
            &[("test:gauged".into(), part(0))],
            &tags,
            "block",
        );
        let kept: Vec<usize> = table["test:gauge"].iter().map(|a| a.mod_index).collect();
        assert_eq!(kept, [0]);
    }

    #[test]
    pub(crate) fn a_mods_tag_part_registered_first_runs_before_its_key_part() {
        let mut r = super::PartRegistrar::new("registrar-test");
        (&mut r as &mut dyn Registrar).block_part_tag("test:gauged", TagGauge);
        let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
        (&mut r as &mut dyn Registrar).block_part(block_key, Gauge);
        let tags = HashMap::from([(
            "test:gauged".to_owned(),
            HashSet::from(["test:gauge".to_owned()]),
        )]);
        super::add_tagged(&mut r.parts.blocks, &r.tags.blocks, &tags, "block");
        let order: Vec<&str> = r.parts.blocks["test:gauge"]
            .iter()
            .map(|a| a.type_name)
            .collect();
        assert_eq!(order, [type_name::<TagGauge>(), type_name::<Gauge>()]);
    }

    #[test]
    pub(crate) fn the_same_part_twice_on_one_block_is_refused() {
        struct Other;
        #[mod_api::horizon_part]
        impl BlockPart for Other {}
        let mut r = super::PartRegistrar::new("first");
        let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
        (&mut r as &mut dyn Registrar).block_part(block_key, Gauge);
        r.mod_id = "second";
        r.mod_index = 1;
        let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
        (&mut r as &mut dyn Registrar).block_part(block_key, Other);
        let block_key = (&mut r as &mut dyn Registrar).block_key("test:gauge");
        (&mut r as &mut dyn Registrar).block_part(block_key, Gauge);
        (&mut r as &mut dyn Registrar).block_key("test:bad name");
        (&mut r as &mut dyn Registrar).block_part_tag("#test:gauged", Other);
        assert_eq!(r.parts.blocks["test:gauge"].len(), 2);
        assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
        assert!(
            r.refused[0].contains("Gauge is attached to test:gauge twice, by first and second")
        );
        assert!(r.refused[1].contains("mod second: invalid registry key"));
        assert!(r.refused[2].contains("write it without #"));
    }
}

/// Registrar operations for blocks.
pub trait BlocksRegistrar {
    /// Request a block by canonical registry id. All mods' declarations are checked at load.
    fn block_key(&mut self, id: &str) -> crate::BlockKey;

    /// Attach `part`, of kind `kind` (its type's name), to block `block`.
    fn attach_block_part(
        &mut self,
        block: crate::BlockKey,
        kind: &'static str,
        part: Arc<dyn BlockPart>,
    );

    /// Attach `part`, of kind `kind`, to every block in tag `tag`.
    fn attach_block_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn BlockPart>);
}

impl BlocksRegistrar for PartRegistrar {
    fn block_key(&mut self, id: &str) -> crate::BlockKey {
        match self
            .registry_keys
            .borrow_mut()
            .blocks
            .request(self.mod_id, id)
        {
            Ok(slot) => crate::BlockKey(slot),
            Err(error) => {
                self.refused.push(error);
                crate::BlockKey(u32::MAX)
            }
        }
    }

    fn attach_block_part(
        &mut self,
        block: crate::BlockKey,
        kind: &'static str,
        part: Arc<dyn BlockPart>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .blocks
            .id(block.0)
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {}: invalid block key slot {}",
                self.mod_id, block.0
            ));
            return;
        };
        self.attach(|p| &mut p.blocks, kind, &id, part);
    }

    fn attach_block_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn BlockPart>) {
        let part = self.tagged(kind, tag, part);
        self.tags.blocks.extend(part);
    }
}
