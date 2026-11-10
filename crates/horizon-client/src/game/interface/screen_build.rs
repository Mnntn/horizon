//! A screen's built state (`Screen.init`): made when the screen opens, remade
//! when the window or the GUI scale resizes it, dropped when it closes.
//!
//! Step 1 of #903 put the seam in: every screen built `BuildLayout::Empty`,
//! with the window's rectangle and nothing a screen drew or hit-tested.
//! Step 2 moves the container screens onto it (`BuildLayout::Container`): the
//! panel, the book and effect panels beside it, the creative tab rows and
//! every slot's hover box are made once here, and the drawing, the hit tests
//! and `screen_exclusion` read them instead of rebuilding them per call.

use horizon_client::inventory_effects::{EffectsLayout, EffectsSpace};
use horizon_client::window::GuiSize;
use mod_api::ScreenView;

use super::{Game, Screen};

/// What the native hooks see of the screen showing (`observe_screen`), kept
/// beside its build so one `ScreenState` holds both.
pub(super) struct ObservedScreen {
    pub(super) id: &'static str,
    /// Not a `Screen` value: the title's own screen or an overlay drawn over
    /// gameplay (sign editor, book, dialog, prompt). `set_screen` never sees
    /// it open or close.
    pub(super) overlay: bool,
    pub(super) menu: Option<String>,
    pub(super) title: Option<std::sync::Arc<mod_api::Text>>,
    pub(super) rect: [f32; 4],
    pub(super) slots: usize,
    pub(super) mouse: [f32; 2],
    pub(super) partial_tick: f32,
}

impl ObservedScreen {
    pub(super) fn view(&self) -> ScreenView<'_> {
        ScreenView::from(mod_api::__private::built::ScreenView {
            id: self.id,
            menu: self.menu.as_deref(),
            title: self.title.as_deref(),
            rect: self.rect,
            slots: self.slots,
            mouse: self.mouse,
            partial_tick: self.partial_tick,
        })
    }
}

/// What a screen's drawing and its input read: its GUI-scaled size, the panel
/// it draws in, and its own widgets as rectangles.
///
/// Built once per open and per resize, never per frame. A screen bumps its own
/// revision when one of its layout inputs moves (Options' page or scroll, a
/// book opening), and the build is remade then.
pub(super) struct ScreenBuild {
    /// Which screen this was built for.
    pub(super) screen: Screen,
    identity: Option<&'static str>,
    /// The GUI-scaled size (`getGuiScaledWidth`/`getGuiScaledHeight`), in GUI pixels.
    pub(super) size: (f32, f32),
    /// The GUI scale (`narration`'s framebuffer space).
    pub(super) scale: f32,
    /// The panel the screen draws (`NativeScreenView.rect`), else the window.
    pub(super) rect: [f32; 4],
    /// The panel's `leftPos`/`topPos`.
    pub(super) origin: (f32, f32),
    /// The screen's own widgets, as rectangles.
    pub(super) layout: BuildLayout,
    /// Panels drawn outside `rect`.
    pub(super) exclusion: Vec<[f32; 4]>,
    /// `screen_init_pre` cancelled the screen's own widgets. Always false
    /// until that hook lands (step 7).
    #[allow(dead_code, reason = "step 7 of #903 reads the cancel")]
    pub(super) cancelled: bool,
}

/// A container screen's layout: the panel it draws in, the panels beside it,
/// and every slot's hover box. Read by its drawing, `slot_at`,
/// `clicked_outside` and `screen_exclusion`; never rebuilt per call.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ContainerLayout {
    pub(super) slot_inputs: super::container_layout::SlotInputs,
    /// `leftPos`, `topPos`, `imageWidth`, `imageHeight` in GUI pixels.
    pub(super) image: [f32; 4],
    /// `getXOrigin`/`getYOrigin`: the player inventory's top-left corner.
    pub(super) origin: (f32, f32),
    /// The open recipe book's panel.
    pub(super) book: Option<[f32; 4]>,
    /// The effect panels' box beside the window.
    pub(super) effects: Option<[f32; 4]>,
    pub(super) shows_effects: bool,
    pub(super) effects_revision: u64,
    /// The `RenderInventoryMobEffects` answer the panel box was laid out with.
    pub(super) effects_layout: EffectsLayout,
    /// The creative tab rows above and below the window.
    pub(super) tabs: Option<[[f32; 4]; 2]>,
    /// Every slot's `isHovering` box (the 16x16 slot grown by one pixel on
    /// every side) and its slot id, in `slots()` order.
    pub(super) slots: Vec<(usize, [f32; 4])>,
}

impl ContainerLayout {
    /// The areas the screen draws outside `image`, in the order the screen
    /// draws them: the creative tab rows then the effect panels, or the
    /// recipe book then the effect panels.
    pub(super) fn exclusion(&self) -> Vec<[f32; 4]> {
        let mut areas = Vec::new();
        match self.tabs {
            Some([above, below]) => {
                areas.push(above);
                areas.push(below);
            }
            None => areas.extend(self.book),
        }
        areas.extend(self.effects);
        areas
    }
}

/// A screen's own widgets, as rectangles.
///
/// Step 2 fills `Container` for the inventory, the creative inventory and
/// every container or station window; `Options`, `Pause` and `Chat` arrive
/// with the screens that fill them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) enum BuildLayout {
    /// Not migrated yet: the build has the window's rectangle only.
    #[default]
    Empty,
    /// A container screen: its panel, the panels beside it, its slot boxes.
    Container(ContainerLayout),
}

impl ScreenBuild {
    /// The build for `screen` at `gui`. A screen without a `ContainerLayout`
    /// knows no panel and no widgets, so its rectangle is the window and its
    /// origin is zero.
    fn new(
        screen: Screen,
        identity: Option<&'static str>,
        gui: GuiSize,
        container: Option<ContainerLayout>,
    ) -> Self {
        let size = (gui.size[0] as f32, gui.size[1] as f32);
        let (rect, origin, layout, exclusion) = match container {
            Some(layout) => {
                let exclusion = layout.exclusion();
                (
                    layout.image,
                    layout.origin,
                    BuildLayout::Container(layout),
                    exclusion,
                )
            }
            None => (
                [0.0, 0.0, size.0, size.1],
                (0.0, 0.0),
                BuildLayout::Empty,
                Vec::new(),
            ),
        };
        Self {
            screen,
            identity,
            size,
            scale: gui.scale as f32,
            rect,
            origin,
            layout,
            exclusion,
            cancelled: false,
        }
    }

    /// The build's container layout, when it has one at this size.
    pub(super) fn container(&self, width: f32, height: f32) -> Option<&ContainerLayout> {
        if self.size != (width, height) {
            return None;
        }
        match &self.layout {
            BuildLayout::Container(layout) => Some(layout),
            BuildLayout::Empty => None,
        }
    }
}

/// What the game knows about the screen showing: the snapshot the native hooks
/// see (`ObservedScreen`) and the build the screen's drawing will read.
#[derive(Default)]
pub(super) struct ScreenState {
    pub(super) observed: Option<ObservedScreen>,
    pub(super) build: ScreenBuildState,
}

/// One frame's `RenderInventoryMobEffects` answer and what it was taken for.
///
/// The display's change builds twice in one frame (`apply_ui_settings` then
/// `observe_screen`); the hook's contract is one dispatch a frame, so the
/// second build reuses this answer instead of asking again.
#[derive(Clone, Copy, PartialEq)]
struct EffectsMemo {
    frame: u64,
    screen: Screen,
    space: EffectsSpace,
    answer: EffectsLayout,
}

/// The open screen's build and what it was made from.
#[derive(Default)]
pub(super) struct ScreenBuildState {
    current: Option<ScreenBuild>,
    /// The frame's effects answer, while this frame asked for one.
    effects: std::cell::Cell<Option<EffectsMemo>>,
    #[cfg(test)]
    builds: u32,
}

impl ScreenBuildState {
    /// A fresh overlay instance must build even when its kind is unchanged.
    pub(super) fn reopened(&mut self) {
        self.current = None;
    }

    /// The answer this frame already took for `screen` in `space`, so a
    /// second build of the same frame asks the mods nothing.
    pub(super) fn effects_answer(
        &self,
        frame: u64,
        screen: Screen,
        space: EffectsSpace,
    ) -> Option<EffectsLayout> {
        self.effects
            .get()
            .filter(|memo| memo.frame == frame && memo.screen == screen && memo.space == space)
            .map(|memo| memo.answer)
    }

    /// Keeps this frame's answer for a second build in the same frame.
    pub(super) fn keep_effects_answer(
        &self,
        frame: u64,
        screen: Screen,
        space: EffectsSpace,
        answer: EffectsLayout,
    ) {
        self.effects.set(Some(EffectsMemo {
            frame,
            screen,
            space,
            answer,
        }));
    }

    /// Reconciles the build with what shows now: a different screen or a
    /// different GUI size makes it, the same one keeps it, and nothing shown
    /// drops it. A call that changes nothing allocates nothing, so the frame
    /// path checks it every frame, and a menu swap on the same screen keeps it.
    ///
    /// `container` is supplied only when the frame's allocation-free input
    /// comparison found a change. The same screen can keep its panel while its layout
    /// moves under it (a book opens, the effects answer changes, a creative
    /// tab swaps the slots), so a different layout remakes the build in place.
    pub(super) fn sync(
        &mut self,
        screen: Screen,
        shown: bool,
        identity: Option<&'static str>,
        gui: GuiSize,
        container: Option<ContainerLayout>,
    ) {
        if !shown {
            self.current = None;
            return;
        }
        let stale = self.current.as_ref().is_none_or(|build| {
            build.screen != screen
                || build.identity != identity
                || build.size != (gui.size[0] as f32, gui.size[1] as f32)
                || build.scale != gui.scale as f32
        });
        if stale {
            self.current = Some(ScreenBuild::new(screen, identity, gui, container));
            #[cfg(test)]
            {
                self.builds += 1;
            }
            return;
        }
        let Some(layout) = container else {
            return;
        };
        let build = self.current.as_mut().expect("shown");
        if matches!(&build.layout, BuildLayout::Container(built) if *built == layout) {
            return;
        }
        let exclusion = layout.exclusion();
        build.rect = layout.image;
        build.origin = layout.origin;
        build.layout = BuildLayout::Container(layout);
        build.exclusion = exclusion;
        #[cfg(test)]
        {
            self.builds += 1;
        }
    }
}

impl Game<'_> {
    /// The framebuffer's size and its GUI-scaled size and scale, as the HUD
    /// computes them (`HudBatch::new`). Never asks for the renderer: before one
    /// exists, the window's own size serves.
    fn gui_frame_now(&self) -> ([u32; 2], GuiSize) {
        let framebuffer = self.renderer.as_ref().map_or_else(
            || {
                self.window.as_ref().map_or([1280, 720], |window| {
                    let size = window.inner_size();
                    [size.width, size.height]
                })
            },
            |renderer| [renderer.config.width, renderer.config.height],
        );
        let inputs = self.settings.gui_scale_inputs();
        let gui = GuiSize::new(framebuffer, inputs.requested, inputs.unicode);
        (framebuffer, gui)
    }

    /// `Screen.init`/`Screen.resize` for the game's open screen: builds it
    /// when it opens, remakes it when the window or the GUI scale changed it,
    /// and drops it when it closes. A frame that changes neither builds
    /// nothing, so `observe_screen` calls it every frame.
    pub(super) fn build_screen(&mut self) {
        let shown =
            self.screen != Screen::Playing || self.title.is_some() || self.screen_over_playing();
        let (framebuffer, gui) = self.gui_frame_now();
        let identity = self
            .title
            .as_ref()
            .map(|title| title.mode.id())
            .or_else(|| {
                self.screen_over_playing()
                    .then(|| self.observed_screen_view().map(|view| view.id))
                    .flatten()
            });
        // Validate the assigned mod book before its flags can affect any
        // cached origin, slot rectangles or exclusion areas.
        if self.native_screen_book().is_some() {
            self.ensure_mod_book_displays();
        }
        // The container screens' layout is a build input: the frame's own
        // state, and the mods' effects answer, compared here so the panels and
        // the exclusion never disagree by a frame.
        let container = (shown && matches!(self.screen, Screen::Inventory | Screen::Creative))
            .then(|| {
                let batch = horizon_client::hud::HudBatch::new(
                    framebuffer,
                    self.settings.gui_scale_inputs(),
                );
                self.container_layout_inputs(&batch)
            });
        let container = container.and_then(|inputs| {
            let unchanged = self
                .screen_state
                .build
                .current
                .as_ref()
                .is_some_and(|build| {
                    build.screen == self.screen
                        && build.identity == identity
                        && build.scale == gui.scale as f32
                        && build
                            .container(gui.size[0] as f32, gui.size[1] as f32)
                            .is_some_and(|old| {
                                old.image == inputs.image
                                    && old.origin == inputs.origin
                                    && old.book == inputs.book
                                    && old.shows_effects == inputs.shows_effects
                                    && old.effects_revision == inputs.effects_revision
                                    && old.effects_layout == inputs.effects_layout
                                    && old.tabs == inputs.tabs
                                    && old.slot_inputs == inputs.slot_inputs
                            })
                });
            (!unchanged).then(|| {
                self.build_container_layout(
                    &horizon_client::hud::HudBatch::new(
                        framebuffer,
                        self.settings.gui_scale_inputs(),
                    ),
                    inputs,
                )
            })
        });
        self.screen_state
            .build
            .sync(self.screen, shown, identity, gui, container);
    }

    /// The build's container layout, when it holds `batch`'s size.
    pub(super) fn built_container(
        &self,
        batch: &horizon_client::hud::HudBatch,
    ) -> Option<&ContainerLayout> {
        self.screen_state
            .build
            .current
            .as_ref()?
            .container(batch.width, batch.height)
    }
}

#[cfg(test)]
mod tests {
    use super::{BuildLayout, ContainerLayout, EffectsLayout, ScreenBuildState};
    use crate::game::Screen;
    use horizon_client::hud::{GuiScale, HudBatch};
    use horizon_client::window::GuiSize;

    #[test]
    #[ignore = "needs prepared resources and isolated homes"]
    fn production_screen_build_geometry() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.screen = Screen::Inventory;
            game.container = None;
            game.craft_size = 2;
            game.title = None;
            game.potions.effects.list = vec![horizon_client::effects::EffectInstance::new(
                "speed", 200, 0,
            )];
            let batch = HudBatch::new(
                [320, 240],
                GuiScale {
                    requested: 1,
                    unicode: false,
                },
            );
            let layout = game.container_layout(&batch);
            assert_eq!(layout.origin, (72.0, 37.0));
            assert_eq!(layout.effects, Some([250.0, 37.0, 32.0, 32.0]));
            game.screen = Screen::Creative;
            let layout = game.container_layout(&batch);
            assert_eq!(
                layout.tabs,
                Some([[62.0, 24.0, 195.0, 28.0], [62.0, 188.0, 195.0, 28.0]])
            );
            game.build_screen();
            let before = game.screen_state.build.builds;
            let constructions =
                crate::game::container_layout::LAYOUT_CONSTRUCTIONS.with(|count| count.get());
            let slots = match &game.screen_state.build.current.as_ref().unwrap().layout {
                BuildLayout::Container(layout) => layout.slots.as_ptr(),
                _ => panic!("container"),
            };
            let positions = crate::game::inventory_slots::SLOT_POSITIONS.with(|count| count.get());
            game.build_screen();
            assert_eq!(
                crate::game::inventory_slots::SLOT_POSITIONS.with(|count| count.get()),
                positions
            );
            assert_eq!(game.screen_state.build.builds, before);
            assert_eq!(
                crate::game::container_layout::LAYOUT_CONSTRUCTIONS.with(|count| count.get()),
                constructions
            );
            let BuildLayout::Container(layout) =
                &game.screen_state.build.current.as_ref().unwrap().layout
            else {
                panic!("container")
            };
            assert_eq!(layout.slots.as_ptr(), slots);
            // Each changed input builds once; its next frame does no slot work.
            let check = |game: &mut crate::game::Game<'_>| {
                let builds = crate::game::container_layout::LAYOUT_CONSTRUCTIONS.with(|c| c.get());
                game.build_screen();
                assert_eq!(
                    crate::game::container_layout::LAYOUT_CONSTRUCTIONS.with(|c| c.get()),
                    builds + 1
                );
                let positions = crate::game::inventory_slots::SLOT_POSITIONS.with(|c| c.get());
                game.build_screen();
                assert_eq!(
                    crate::game::inventory_slots::SLOT_POSITIONS.with(|c| c.get()),
                    positions
                );
            };
            game.settings.gui_scale = 1;
            check(game);
            game.screen = Screen::Inventory;
            check(game);
            game.level.recipe_book.open[0] = true;
            check(game);
            game.potions.effects.list.clear();
            check(game);
            game.craft_size = 3;
            check(game);
            game.station = Some(crate::game::stations::Station::new_kind(
                crate::game::stations::Kind::Merchant,
                [0, 0, 0],
            ));
            check(game);
            game.station = None;
            game.screen = Screen::Creative;
            check(game);
            let inventory = game
                .creative_screen
                .tab_headers()
                .iter()
                .position(|tab| tab.inventory)
                .unwrap();
            game.creative_screen.select(inventory);
            check(game);
        });
    }

    fn gui(framebuffer: [u32; 2], scale: u32) -> GuiSize {
        GuiSize::new(framebuffer, scale, false)
    }

    /// A container layout with only the fields a case cares about set.
    fn container(
        book: Option<[f32; 4]>,
        effects: Option<[f32; 4]>,
        effects_layout: EffectsLayout,
        tabs: Option<[[f32; 4]; 2]>,
    ) -> ContainerLayout {
        ContainerLayout {
            slot_inputs: Default::default(),
            image: [10.0, 20.0, 176.0, 166.0],
            origin: (10.0, 20.0),
            book,
            effects,
            shows_effects: effects.is_some(),
            effects_revision: 0,
            effects_layout,
            tabs,
            slots: vec![(0, [9.0, 141.0, 18.0, 18.0])],
        }
    }

    /// One build per open and per resize; none for a frame that changed
    /// nothing, and none for a menu swapped on the same screen.
    #[test]
    fn a_build_per_open_and_resize_only() {
        let mut state = ScreenBuildState::default();
        let small = gui([854, 480], 0);
        state.sync(Screen::Inventory, true, None, small, None);
        assert_eq!(state.builds, 1);
        // Frames pass with the same screen and size.
        for _ in 0..3 {
            state.sync(Screen::Inventory, true, None, small, None);
        }
        assert_eq!(state.builds, 1);
        // A menu swapped on the same screen keeps the build.
        state.sync(Screen::Inventory, true, None, small, None);
        assert_eq!(state.builds, 1);
        // A window resize rebuilds once.
        let large = gui([1920, 1080], 0);
        state.sync(Screen::Inventory, true, None, large, None);
        assert_eq!(state.builds, 2);
        state.sync(Screen::Inventory, true, None, large, None);
        assert_eq!(state.builds, 2);
        // A GUI scale change rebuilds once.
        let scaled = GuiSize::new([1920, 1080], 3, false);
        state.sync(Screen::Inventory, true, None, scaled, None);
        assert_eq!(state.builds, 3);
        // A different screen opens.
        state.sync(Screen::Paused, true, None, scaled, None);
        assert_eq!(state.builds, 4);
    }

    #[test]
    fn distinct_title_and_overlay_identities_rebuild_at_the_same_size() {
        let mut state = ScreenBuildState::default();
        let size = gui([854, 480], 0);
        state.sync(Screen::Playing, true, Some("title"), size, None);
        assert_eq!(state.builds, 1);
        state.sync(Screen::Playing, true, Some("multiplayer"), size, None);
        assert_eq!(state.builds, 2);
        state.sync(Screen::Playing, true, Some("sign_editor"), size, None);
        assert_eq!(state.builds, 3);
        state.sync(Screen::Playing, true, Some("book_view"), size, None);
        assert_eq!(state.builds, 4);
        state.sync(Screen::Playing, true, Some("book_view"), size, None);
        assert_eq!(state.builds, 4);
        state.reopened();
        state.sync(Screen::Playing, true, Some("book_view"), size, None);
        assert_eq!(state.builds, 5);
    }

    #[test]
    fn closing_drops_the_build() {
        let mut state = ScreenBuildState::default();
        let size = gui([1280, 720], 0);
        state.sync(Screen::Paused, true, None, size, None);
        assert!(state.current.is_some());
        state.sync(Screen::Playing, false, None, size, None);
        assert!(state.current.is_none());
        assert_eq!(state.builds, 1);
        // Nothing open builds nothing.
        state.sync(Screen::Playing, false, None, size, None);
        assert_eq!(state.builds, 1);
    }

    /// A title or an overlay shows with `Screen::Playing` under it: it is still
    /// something to build for.
    #[test]
    fn a_title_or_overlay_over_playing_builds() {
        let mut state = ScreenBuildState::default();
        state.sync(Screen::Playing, true, None, gui([854, 480], 0), None);
        assert_eq!(state.builds, 1);
        assert_eq!(state.current.as_ref().unwrap().screen, Screen::Playing);
    }

    /// The build's size and scale are the ones the HUD draws at
    /// (`HudBatch::new`), at every window size and GUI scale.
    #[test]
    fn the_build_is_the_size_and_scale_the_hud_draws_at() {
        for (window, scale, unicode) in [
            ([320, 240], 0, false),
            ([854, 480], 1, false),
            ([1280, 720], 2, true),
            ([1920, 1080], 3, false),
            ([2560, 1440], 4, false),
            ([3840, 2160], 0, true),
            ([1279, 719], 2, false),
        ] {
            let gui = GuiSize::new(window, scale, unicode);
            let batch = HudBatch::new(
                window,
                GuiScale {
                    requested: scale,
                    unicode,
                },
            );
            let mut state = ScreenBuildState::default();
            state.sync(Screen::Inventory, true, None, gui, None);
            let build = state.current.as_ref().unwrap();
            assert_eq!(
                build.size,
                (batch.width, batch.height),
                "{window:?} at {scale}"
            );
            assert_eq!(build.scale, batch.scale, "{window:?} at {scale}");
            assert_eq!(build.rect, [0.0, 0.0, batch.width, batch.height]);
            assert!(matches!(build.layout, BuildLayout::Empty));
        }
    }

    /// The build's exclusion is the panels that build draws, and it follows
    /// them in the same call: the effects appearing or the mod's answer moving
    /// them never leaves the exclusion a frame behind.
    #[test]
    fn the_exclusion_is_the_panels_the_build_draws() {
        let mut state = ScreenBuildState::default();
        let size = gui([854, 480], 0);
        let book = [100.0, 20.0, 147.0, 166.0];
        let mut layout = container(Some(book), None, EffectsLayout::vanilla(200), None);
        state.sync(Screen::Inventory, true, None, size, Some(layout.clone()));
        assert_eq!(state.builds, 1);
        assert_eq!(state.current.as_ref().unwrap().exclusion, vec![book]);
        // A potion takes effect: from this call on the book and the panel.
        let panel = [200.0, 20.0, 100.0, 32.0];
        layout.effects = Some(panel);
        state.sync(Screen::Inventory, true, None, size, Some(layout.clone()));
        assert_eq!(state.builds, 2);
        assert_eq!(state.current.as_ref().unwrap().exclusion, vec![book, panel]);
        // The mod answers compact and nudged: the exclusion follows at once.
        let moved = [205.0, 20.0, 32.0, 32.0];
        layout.effects = Some(moved);
        layout.effects_layout = EffectsLayout {
            compact: true,
            offset: 5,
            hidden: false,
        };
        state.sync(Screen::Inventory, true, None, size, Some(layout.clone()));
        assert_eq!(state.builds, 3);
        assert_eq!(state.current.as_ref().unwrap().exclusion, vec![book, moved]);
        // The panel the drawing reads is the one the exclusion named.
        assert_eq!(
            state.current.as_ref().unwrap().origin,
            (layout.origin.0, layout.origin.1)
        );
        assert_eq!(
            state.current.as_ref().unwrap().rect,
            [10.0, 20.0, 176.0, 166.0]
        );
        // An unchanged frame keeps the build.
        state.sync(Screen::Inventory, true, None, size, Some(layout));
        assert_eq!(state.builds, 3);
    }

    /// The creative screen's exclusion is its two tab rows over and under the
    /// window, then the effects; the same layout keeps the same build.
    #[test]
    fn the_creative_tab_rows_are_excluded_above_and_below() {
        let mut state = ScreenBuildState::default();
        let size = gui([1280, 720], 0);
        let tabs = Some([[0.0, 0.0, 195.0, 28.0], [0.0, 136.0, 195.0, 28.0]]);
        let layout = container(None, None, EffectsLayout::vanilla(100), tabs);
        state.sync(Screen::Creative, true, None, size, Some(layout.clone()));
        assert_eq!(
            state.current.as_ref().unwrap().exclusion,
            vec![[0.0, 0.0, 195.0, 28.0], [0.0, 136.0, 195.0, 28.0]]
        );
        state.sync(Screen::Creative, true, None, size, Some(layout));
        assert_eq!(state.builds, 1);
    }

    /// The display's change builds twice in one frame (`apply_ui_settings`
    /// then `observe_screen`): the effects hook still fires once, and the next
    /// frame fires it again. A mod's once-per-frame side effect stays once.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_effects_answer_is_taken_once_a_frame() {
        use mod_api::{ClientHost, Hooks, ModEntry, ScreenRenderInventoryMobEffectsEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        /// A mod listening to the effects hook, counting its dispatches.
        struct Effects(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Effects {
            /// Counts the dispatch the build asks for.
            fn screen_render_inventory_mob_effects(
                &self,
                _: &dyn ClientHost,
                _: &mut ScreenRenderInventoryMobEffectsEvent<'_>,
            ) -> mod_api::Flow {
                self.0.fetch_add(1, Ordering::Relaxed);
                mod_api::Flow::Continue
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "effects",
            Box::new(Effects(Arc::clone(&calls))),
        )]
        .into();
        mod_api::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
                game.screen = Screen::Inventory;
                game.container = None;
                game.craft_size = 2;
                game.potions.effects.list = vec![horizon_client::effects::EffectInstance::new(
                    "speed", 200, 0,
                )];
                game.frames = 11;
                game.build_screen();
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    1,
                    "the frame's first build asks the mods"
                );
                game.build_screen();
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    1,
                    "a second build in the same frame reuses the answer"
                );
                game.frames = 12;
                game.build_screen();
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    2,
                    "the next frame asks again"
                );
                assert_eq!(calls.load(Ordering::Relaxed), 2, "and only once");
            });
        });
    }
}
