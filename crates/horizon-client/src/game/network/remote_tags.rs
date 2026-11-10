//! The server's tags on this client (`ClientPacketListener.handleUpdateTags`, and configuration's
//! `RegistryDataCollector`): what native mods read of them, and their notice. The client's own
//! rules keep playing with its tag files.
use super::Game;
use horizon_client::items::{Items, TagRegistry};
use horizon_protocol::{item::Names, packets::configuration::UpdateTags, registry::SyncedNames};
use std::sync::Arc;

/// The registries whose tags the tag helpers read.
const READ_REGISTRIES: [(&str, TagRegistry); 3] = [
    ("minecraft:block", TagRegistry::Block),
    ("minecraft:item", TagRegistry::Item),
    ("minecraft:fluid", TagRegistry::Fluid),
];

impl Game<'_> {
    /// `ClientboundUpdateTagsPacket` in play.
    pub(super) fn remote_update_tags(&mut self, tags: &UpdateTags) {
        if let Some(r) = self.remote.as_mut() {
            Arc::make_mut(&mut r.registries).add_tags(tags);
        }
        self.install_remote_tags();
    }

    /// Make the server's tags what native mods read, then tell them. A connection whose
    /// configuration sent no tags has nothing to apply, and mods that aren't there to read
    /// them nothing to resolve.
    pub(super) fn install_remote_tags(&mut self) {
        if mod_api::__private::mods().is_empty() {
            return;
        }
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if r.registries.tags.is_empty() {
            r.tags = None;
            return;
        }
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        let mut items = Items::empty();
        for (key, registry) in READ_REGISTRIES {
            let Some(tags) = r.registries.tags.get(key) else {
                continue;
            };
            let members = |ids: &[i32]| ids.iter().filter_map(|&id| names.name(key, id)).collect();
            let tags = tags.iter().map(|(tag, ids)| (tag.clone(), members(ids)));
            items.set_tags(registry, tags.collect());
        }
        super::class_tags::install(&mut items, self.res.facts());
        super::native_world_read::install_block_tags(self.block_tag_sets(&items));
        if let Some(r) = self.remote.as_mut() {
            r.tags = Some(items);
        }
        self.native_tags_updated(mod_api::TagsUpdateCause::ClientPacketReceived {
            integrated_server: self.integrated.is_some(),
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_client_game;
    use horizon_protocol::packets::{
        configuration::{RegistryTags, TagEntry, UpdateTags},
        play::{self, Clientbound as C},
    };
    use horizon_protocol::registry::Builtin;
    use mod_api::__private::part_tests::block_key;
    use mod_api::{Hooks, HostSide, Identifier, ModEntry, TagsUpdateCause, TagsUpdatedEvent};
    use std::sync::{Arc, Mutex};

    /// What a listener read of the tags during its notice.
    #[derive(Debug, PartialEq)]
    struct Seen {
        cause: TagsUpdateCause,
        items: Option<Vec<String>>,
        fluids: Option<usize>,
        stone_in_block_tag: Option<bool>,
        dirt_in_block_tag: Option<bool>,
        stick_in_item_tag: Option<bool>,
        lava_in_fluid_tag: Option<bool>,
    }

    struct Probe(Arc<Mutex<Vec<Seen>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn tags_updated(&self, host: HostSide<'_>, event: &mut TagsUpdatedEvent) {
            let HostSide::Client(host) = host else {
                panic!("the server's tags are a client's to apply");
            };
            let host: &dyn mod_api::Host = &*host;
            let tags = host.tags();
            let key = |tag| tags.key(&Identifier::parse(tag).unwrap()).unwrap();
            let stick = host.item_key(&Identifier::parse("minecraft:stick").unwrap());
            let items = tags.items(key("test:items"));
            self.0.lock().unwrap().push(Seen {
                cause: event.cause,
                items: items.map(|keys| keys.into_iter().map(|key| key.id(host)).collect()),
                fluids: tags.fluids(key("test:fluids")).map(|keys| keys.len()),
                stone_in_block_tag: host.block_in_tag(block_key("minecraft:stone"), "test:blocks"),
                dirt_in_block_tag: host.block_in_tag(block_key("minecraft:dirt"), "test:blocks"),
                stick_in_item_tag: tags.is_in_tag(stick.unwrap(), key("test:items")),
                lava_in_fluid_tag: host.fluid_in_tag("minecraft:lava", "test:fluids"),
            });
        }
    }

    fn update(builtin: &Builtin, tags: &[(&str, &str, &[&str])]) -> UpdateTags {
        let id = |registry: &str, name: &str| builtin.get(registry).unwrap().id(name).unwrap();
        UpdateTags {
            registries: tags
                .iter()
                .map(|(registry, tag, members)| RegistryTags {
                    registry: (*registry).into(),
                    tags: vec![TagEntry {
                        name: (*tag).into(),
                        ids: members.iter().map(|name| id(registry, name)).collect(),
                    }],
                })
                .collect(),
        }
    }

    fn login() -> C {
        C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        })
    }

    fn names(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(|name| (*name).to_owned()).collect())
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_client_notice_reads_each_applied_set_of_server_tags() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("tags_probe", Box::new(Probe(seen.clone())))].into();
        mod_api::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "Steve").unwrap();
                let builtin = game.remote.as_ref().unwrap().builtin.clone();

                // Configuration's tags wait for the login, which binds the client's blocks.
                let registries = &mut game.remote.as_mut().unwrap().registries;
                Arc::make_mut(registries).add_tags(&update(
                    &builtin,
                    &[
                        (
                            "minecraft:block",
                            "test:blocks",
                            &["minecraft:stone", "minecraft:dirt"],
                        ),
                        ("minecraft:item", "test:items", &["minecraft:stick"]),
                        ("minecraft:fluid", "test:fluids", &["minecraft:lava"]),
                    ],
                ));
                assert!(seen.lock().unwrap().is_empty());
                assert!(game.native_tag_items().is_none());

                game.handle_packet(login()).unwrap();
                let received = TagsUpdateCause::ClientPacketReceived {
                    integrated_server: false,
                };
                assert_eq!(
                    *seen.lock().unwrap(),
                    [Seen {
                        cause: received,
                        items: names(&["minecraft:stick"]),
                        fluids: Some(1),
                        stone_in_block_tag: Some(true),
                        dirt_in_block_tag: Some(true),
                        stick_in_item_tag: Some(true),
                        lava_in_fluid_tag: Some(true),
                    }]
                );

                // An update in play replaces the registries it names and keeps the others.
                game.handle_packet(C::UpdateTags(update(
                    &builtin,
                    &[
                        ("minecraft:block", "test:blocks", &["minecraft:dirt"]),
                        ("minecraft:item", "test:other", &["minecraft:stick"]),
                    ],
                )))
                .unwrap();
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 2);
                assert_eq!(
                    seen[1],
                    Seen {
                        cause: received,
                        items: Some(Vec::new()),
                        fluids: Some(1),
                        stone_in_block_tag: Some(false),
                        dirt_in_block_tag: Some(true),
                        stick_in_item_tag: Some(false),
                        lava_in_fluid_tag: Some(true),
                    }
                );
            });
        });
    }
}
