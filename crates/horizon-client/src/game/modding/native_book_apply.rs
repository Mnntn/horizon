//! Authoritative placement: detached calculation, journaled writes, then publication.
use super::{Game, native_book_backings::Backings, native_book_scope::MenuSnapshot};
use horizon_client::{
    lan::menu::ModCraft,
    menu::{
        Env, Kind, Menu,
        place::{
            PostPlace,
            custom_plan::{MenuPlacement, PlacementRefusal},
        },
    },
    recipe_book::placement_sync as sync,
};

fn refusal(reason: PlacementRefusal) -> sync::Refusal {
    use sync::Refusal as R;
    match reason {
        PlacementRefusal::NotPlaceable => R::NotPlaceable,
        PlacementRefusal::InvalidLayout => R::Invalid,
        PlacementRefusal::Unavailable => R::Unavailable,
        PlacementRefusal::Denied => R::Denied,
        PlacementRefusal::NoRoom => R::NoRoom,
        PlacementRefusal::Changed => R::Changed,
    }
}
impl Game<'_> {
    pub(super) fn server_mod_book_place(
        &mut self,
        conn: u32,
        selection: &sync::Selection,
        use_max: bool,
    ) -> sync::Outcome {
        match self.apply_mod_book_place(conn, selection, use_max) {
            Ok(post) => {
                if post == PostPlace::Ghost {
                    sync::Outcome::MissingIngredients
                } else {
                    sync::Outcome::Placed
                }
            }
            Err(reason) => sync::Outcome::Refused(reason),
        }
    }
    fn apply_mod_book_place(
        &mut self,
        conn: u32,
        selection: &sync::Selection,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        if !self.transactions.is_empty() {
            return Err(R::Denied);
        }
        let settings = self
            .native_net
            .book_connections
            .get(&conn)
            .and_then(|connection| connection.authority.as_ref())
            .ok_or(R::Unsupported)?;
        if settings.epoch != selection.catalog.epoch
            || !settings.contains(selection.catalog.book.as_str())
        {
            return Err(R::Stale);
        }
        let authority = self
            .native_net
            .placement
            .servers
            .get_mut(&conn)
            .ok_or(R::Unsupported)?;
        authority.peer.place(selection)?;
        if !authority
            .displays
            .get(selection.display.recipe.as_str())
            .is_some_and(|count| selection.display.source < *count)
        {
            return Err(R::UnknownDisplay);
        }
        self.ensure_mod_book_displays();
        let index = self
            .recipe_tables
            .books
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .ok_or(R::Invalid)?;
        if index.generation() != selection.catalog.generation {
            return Err(R::Stale);
        }
        let plan = index
            .placement(selection.display.recipe.as_str())
            .map_err(refusal)?
            .clone();
        let before = self.mod_book_menu_snapshot(
            conn,
            selection.catalog.window,
            selection.catalog.menu.as_str(),
        )?;
        if before.instance != selection.catalog.instance {
            return Err(R::Stale);
        }
        if !before.known.contains(selection.display.recipe.as_str()) {
            return Err(R::Denied);
        }
        let seal = self.transaction_open();
        let result = self.stage_mod_book_place(&before, selection, &plan, use_max);
        self.transaction_close(&seal, result.is_ok());
        result
    }
    fn mod_book_placement_context(
        &self,
        before: &MenuSnapshot,
        selection: &sync::Selection,
    ) -> bool {
        self.transactions.len() == 1
            && self.mod_book_menu_unchanged(before)
            && self
                .native_net
                .placement
                .servers
                .get(&before.conn)
                .is_some_and(|server| server.peer.owns(&selection.catalog))
            && self
                .native_net
                .book_connections
                .get(&before.conn)
                .and_then(|connection| connection.authority.as_ref())
                .is_some_and(|authority| {
                    authority.epoch == selection.catalog.epoch
                        && authority.contains(selection.catalog.book.as_str())
                })
    }
    fn stage_mod_book_place(
        &mut self,
        before: &MenuSnapshot,
        selection: &sync::Selection,
        plan: &MenuPlacement,
        use_max: bool,
    ) -> Result<PostPlace, sync::Refusal> {
        use sync::Refusal as R;
        let mut graph = Backings::capture(self, before, plan)?;
        let gi = self
            .lan
            .as_ref()
            .and_then(|host| {
                host.hub
                    .guests
                    .iter()
                    .position(|guest| guest.conn == before.conn)
            })
            .ok_or(R::Changed)?;
        let horizon_client::lan::menu::Slots::Block(pos) = before.slots else {
            return Err(R::Unavailable);
        };
        let sources = self.lan_recipe_sources(gi, before.kind, pos);
        if !self.mod_book_placement_context(before, selection) || !graph.unchanged(self) {
            return Err(R::Changed);
        }
        let stocks = graph.add_sources(self, &sources)?;
        let host = self.lan.as_ref().ok_or(R::Changed)?;
        let guest = host.hub.guest(before.conn).ok_or(R::Changed)?;
        let kind = Kind::Mod(before.kind);
        let mut menu = host
            .play
            .get(&before.conn)
            .and_then(|play| play.crafting.machine.as_ref())
            .filter(|(window, menu)| *window == before.window && menu.kind() == kind)
            .map_or_else(|| Box::new(Menu::new(kind)), |(_, menu)| menu.clone());
        menu.reset_mod_visibility(&before.hidden);
        menu.set_registry_id(Some(before.kind.id));
        super::lan_crafting::load_player(&mut menu, guest);
        menu.set_container_limits(graph.limits());
        menu.sources = stocks;
        for (slot, stack) in graph.main.iter().enumerate() {
            menu.load_slot(slot, stack.clone());
        }
        let known = guest.recipe_book.clone();
        let limited = host.hub.limited_crafting;
        let name = guest.name.clone();
        let post = {
            let env = Env {
                player: Some(before.uuid),
                player_entity_id: Some(before.entity_id),
                known_recipes: Some(&known),
                limited_crafting: limited,
                ..self.menu_env()
            };
            menu.place_custom_recipe(&env, plan, use_max, before.game_mode == 1, || {
                self.mod_book_placement_context(before, selection) && graph.unchanged(self)
            })
            .map_err(refusal)?
        };
        if let Some(crafting) = menu.mod_crafting() {
            let mut result = menu.slot(crafting.result).cloned();
            self.native_prepare_item_craft(
                before.uuid,
                &name,
                Some([pos.0, pos.1, pos.2]),
                &mut result,
            );
            if !self.mod_book_placement_context(before, selection) || !graph.unchanged(self) {
                return Err(R::Changed);
            }
            menu.load_slot(crafting.result, result);
        }
        if !self.native_as_player(before.uuid, |game| game.journal_player(before.uuid)) {
            return Err(R::Changed);
        }
        // The guest's fields are back in the hub before comparing or storing them.
        if !self.mod_book_placement_context(before, selection) || !graph.unchanged(self) {
            return Err(R::Changed);
        }
        let changes = graph.changes(&mut menu);
        for key in changes.keys() {
            self.journal_block_entity(key.pos);
        }
        let mut changed = std::collections::BTreeSet::new();
        for (key, stack) in changes {
            let entity = self
                .world
                .block_entities
                .get_mut(&key.pos)
                .ok_or(R::Changed)?;
            let slot = entity.slots_mut().get_mut(key.slot).ok_or(R::Changed)?;
            *slot = stack;
            changed.insert(key.pos);
        }
        for pos in changed {
            self.after_commit(super::transactions::After::Changed(pos));
        }
        for stack in std::mem::take(&mut menu.dropped) {
            self.drop_after_commit(before.uuid, stack);
        }
        let craft = menu.mod_crafting().map(|crafting| ModCraft {
            grid: crafting
                .grid_slots()
                .map(|slot| menu.slot(slot).cloned())
                .collect(),
            result: menu.slot(crafting.result).cloned(),
        });
        let host = self.lan.as_mut().ok_or(R::Changed)?;
        let gi = host
            .hub
            .guests
            .iter()
            .position(|guest| guest.conn == before.conn)
            .ok_or(R::Changed)?;
        // No callbacks or fallible work follow these live-cache assignments.
        menu.sources.clear();
        host.hub.guests.update(gi, |guest| {
            super::lan_crafting::store_player(&mut menu, guest);
            if let Some(opened) = &mut guest.menu {
                opened.craft = craft;
            }
        });
        host.play.entry(before.conn).or_default().crafting.machine = Some((before.window, menu));
        Ok(post)
    }
}

#[cfg(test)]
mod adversarial_review_tests {
    use super::*;
    #[test]
    #[ignore = "prepared resources, isolated review process"]
    fn actual_game_stage_refuses_nested_frame_and_mutated_actor() {
        use horizon_client::{
            containers::{BlockEntity, ModBlockEntity, ModMenuType},
            crafting::{Ingredient, Placement},
            lan::menu::{Menu as Open, Slots},
            recipe_book::{
                settings_connection::Connection, settings_sync::Authority,
                slot_layout::CheckedSlotLayout,
            },
        };
        super::super::native_guest_turn_tests::with_guests(|game| {
            let pos = (3, 64, 5);
            let kind = Box::leak(Box::new(ModMenuType {
                id: "review:menu",
                slots: 3,
                data_slots: 0,
                results: vec![2],
                crafting: None,
                links: None,
            }));
            let mut block = ModBlockEntity::new("review:block".into(), Default::default());
            block.items = vec![
                Some(horizon_client::items::ItemStack::new("minecraft:dirt", 1)),
                None,
                None,
            ];
            game.world
                .block_entities
                .insert(pos, BlockEntity::Mod(Box::new(block)));
            game.world.block_entities.set_mod_containers(
                [(
                    game.world.block(pos.0, pos.1, pos.2).0,
                    std::sync::Arc::new(horizon_client::containers::ModContainer {
                        slots: 3,
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            game.world.modified.clear();
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.menu_generation = 1;
                g.menu = Some(Open {
                    window: 1,
                    kind: 0,
                    registry_id: "review:menu".into(),
                    pos: Some(pos),
                    slots: Slots::Block(pos),
                    size: 3,
                    state: 0,
                    sent: vec![],
                    drag: None,
                    sent_data: vec![],
                    hidden: vec![],
                    mod_type: Some(kind),
                    craft: None,
                    links: vec![],
                });
            });
            let before = game.mod_book_menu_snapshot(12, 1, "review:menu").unwrap();
            let catalog = sync::CatalogIdentity {
                epoch: 1,
                number: 1,
                window: 1,
                nonce: 1,
                instance: 1,
                generation: 1,
                menu: "review:menu".to_owned().try_into().unwrap(),
                book: "review:book".to_owned().try_into().unwrap(),
            };
            let selection = sync::Selection {
                catalog: catalog.clone(),
                number: 2,
                display: sync::DisplayId {
                    recipe: "review:recipe".to_owned().try_into().unwrap(),
                    source: 0,
                },
            };
            let mut server = super::super::native_book_placement::Server::new(1);
            server.peer.hello(1, sync::schema()).unwrap();
            server.peer.publish(catalog).unwrap();
            game.native_net.placement.servers.insert(12, server);
            let mut connection = Connection::default();
            connection.authority = Some(Authority::negotiate(
                1,
                vec!["review:book".into()],
                &["review:book".into()].into_iter().collect(),
            ));
            game.native_net.book_connections.insert(12, connection);
            let layout = CheckedSlotLayout::new(
                mod_api::RecipeBookLayout::Furnace {
                    input: 0,
                    fuel: 1,
                    result: 2,
                },
                3,
            )
            .unwrap();
            let plan = MenuPlacement::new(
                "review:menu".into(),
                Placement::of(
                    vec![Some(Ingredient::of(vec!["minecraft:iron_ore".into()]))],
                    None,
                ),
                &layout,
            )
            .unwrap();
            let root = game.transaction_open();
            assert!(game.mod_book_placement_context(&before, &selection));
            let child = game.transaction_open();
            assert_eq!(
                game.stage_mod_book_place(&before, &selection, &plan, false),
                Err(sync::Refusal::Changed)
            );
            game.transaction_close(&child, false);
            assert!(game.mod_book_placement_context(&before, &selection));
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.health = 1.0);
            assert_eq!(
                game.stage_mod_book_place(&before, &selection, &plan, false),
                Err(sync::Refusal::Changed)
            );
            assert_eq!(
                game.world.block_entities.get(&pos).unwrap().slots()[0]
                    .as_ref()
                    .unwrap()
                    .count,
                1
            );
            assert!(game.world.modified.is_empty());
            game.transaction_close(&root, false);
        });
    }
}
