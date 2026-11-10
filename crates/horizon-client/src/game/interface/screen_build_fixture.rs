//! The Rust side of tools/test_screen_build_parity.py: with
//! `HORIZON_SCREEN_BUILD_FIXTURE=CASES,OUTPUT` the game loads its resources,
//! opens each case's container screen state, builds its layout and prints the
//! panel, the origin, the slot boxes and the slot a cursor point hovers, in
//! the format of `VanillaScreenBuildOracle`.
use super::{Game, Screen};
use horizon_client::{
    Result,
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    hud::{GuiScale, HudBatch},
    recipe_book::BookType,
    window::GuiSize,
};
use serde_json::Value;
use std::io::Write as _;

/// A container screen's block and block entity at the fixture's one position.
const POS: (i32, i32, i32) = (0, 0, 0);

impl Game<'_> {
    /// Run every case of `spec` ("CASES,OUTPUT") and write the results.
    pub(super) fn screen_build_fixture(&mut self, spec: &str) -> Result<()> {
        let (input, output) = spec
            .split_once(',')
            .ok_or("HORIZON_SCREEN_BUILD_FIXTURE=CASES,OUTPUT")?;
        // The container screens read their panel from the block at POS.
        self.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
        let text = std::fs::read_to_string(input)?;
        let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
        for line in text.lines() {
            let case: Value = serde_json::from_str(line)?;
            match self.screen_build_case(&case) {
                Ok(value) => writeln!(out, "{value}")?,
                Err(e) => writeln!(out, "{}", serde_json::json!({"error": e.to_string()}))?,
            }
        }
        Ok(())
    }

    /// One case's screen state (`kind`, rows, recipe book, window, scale) and
    /// the build that shows it: the panel, the origin, the slot boxes and the
    /// slot each cursor point hovers, as `VanillaScreenBuildOracle` prints them.
    fn screen_build_case(&mut self, case: &Value) -> Result<Value> {
        let kind = case["kind"].as_str().ok_or("kind")?;
        let rows = case["rows"].as_u64().unwrap_or(3);
        let book = case["book"].as_bool().unwrap_or(false);
        let window = [
            case["window"][0].as_u64().ok_or("window")? as u32,
            case["window"][1].as_u64().ok_or("window")? as u32,
        ];
        let scale = case["scale"].as_u64().ok_or("scale")? as u32;

        // The case's screen: a survival/creative container in the inventory
        // screen, with the block entity the menu shows.
        self.screen = Screen::Inventory;
        self.title = None;
        self.craft_size = 2;
        self.container = None;
        self.world.block_entities.clear();
        self.level.recipe_book.open = [false; 4];
        let state = match kind {
            "chest" | "hopper" | "shulker" | "furnace" => {
                let (block, slots) = match kind {
                    "chest" => ("minecraft:chest", rows as usize * 9),
                    "hopper" => ("minecraft:hopper", 5),
                    "shulker" => ("minecraft:shulker_box", 27),
                    _ => ("minecraft:furnace", 3),
                };
                let state = self
                    .pack
                    .assets
                    .registry
                    .default_state(block)
                    .ok_or(block)?;
                let entity = if kind == "furnace" {
                    BlockEntity::Furnace(Furnace {
                        kind: FurnaceKind::Furnace,
                        items: [None, None, None],
                        lit_remaining: 0,
                        lit_total: 0,
                        cooking_timer: 0,
                        cooking_total: 0,
                        speed: 1.0,
                        recipes_used: std::collections::HashMap::new(),
                    })
                } else {
                    BlockEntity::Chest(vec![None; slots])
                };
                self.world.block_entities.insert(POS, entity);
                self.container = Some(POS);
                state
            }
            "crafting" => {
                self.craft_size = 3;
                self.pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
                    .ok_or("air")?
            }
            "inventory" => self
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .ok_or("air")?,
            other => return Err(format!("unknown screen {other}").into()),
        };
        self.world.set(POS.0, POS.1, POS.2, state)?;
        let book_type = match kind {
            "furnace" => BookType::Furnace,
            _ => BookType::Crafting,
        };
        self.level.recipe_book.open[book_type.index()] = book;

        let batch = HudBatch::new(
            window,
            GuiScale {
                requested: scale,
                unicode: false,
            },
        );
        let gui = GuiSize::new(window, scale, false);
        let layout = self.container_layout(&batch);
        // The frame's build, the way `observe_screen` makes it: the hit tests
        // and the panel readers below go through it.
        self.screen_state
            .build
            .sync(self.screen, true, None, gui, Some(layout));

        let Some(layout) = self.built_container(&batch) else {
            return Err("no build".into());
        };
        let image = self.container_image(&batch);
        let (ox, oy) = self.inventory_origin(&batch);
        let mut slots: Vec<[i32; 2]> = layout
            .slots
            .iter()
            .map(|(_, r)| [r[0] as i32 + 1, r[1] as i32 + 1])
            .collect();
        slots.sort_unstable();
        let mut hits = Vec::new();
        for point in case["points"].as_array().ok_or("points")? {
            let x = point[0].as_f64().ok_or("point x")? as f32;
            let y = point[1].as_f64().ok_or("point y")? as f32;
            let hit = self.slot_at(&batch, x, y).map(|slot| {
                let rect = layout
                    .slots
                    .iter()
                    .find(|(s, _)| *s == slot)
                    .map(|(_, r)| *r);
                rect.map(|r| serde_json::json!([r[0] as i32 + 1, r[1] as i32 + 1]))
            });
            hits.push(hit.flatten().unwrap_or(Value::Null));
        }
        Ok(serde_json::json!({
            "gui": [gui.scale, gui.size[0], gui.size[1]],
            "image": [image.left, image.top, image.width, image.height],
            "origin": [ox as i32, oy as i32],
            "slots": slots,
            "hits": hits,
        }))
    }
}
