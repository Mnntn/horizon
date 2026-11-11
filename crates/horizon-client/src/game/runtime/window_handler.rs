//! winit's `ApplicationHandler` for the game window: window and device events, shutting down,
//! and the idle step (`about_to_wait`).
use super::*;

impl Game<'_> {
    /// A run that a person's stray key, click or mouse move would spoil: a screenshot sequence, a
    /// `HORIZON_TICK_SCRIPT` scenario or a `record-replay`. While one runs, the keyboard, mouse
    /// and pad at this computer don't reach the game (`locks_out`, `poll_pads`), the window's
    /// focus stays as the run found it (`held_focus`) and its pointer grab stays inside the game
    /// (`internal_grab`).
    pub(super) fn scripted_run(&self) -> bool {
        self.screenshot_sequence.is_some() || self.tick_script.playing() || self.repro.replaying()
    }
}

/// A key, a modifier, a click, the wheel, the pointer or the input method. Resize and close are
/// not, so a person can still close the window. A modifier change counts: `reconcile` would
/// release a Shift or Ctrl the run holds.
fn is_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::Ime(_)
    )
}

/// A person's input during a scripted run is dropped. What the run itself sends (the control
/// socket, a replay, the game's own) still plays.
fn locks_out(origin: input_trace::InputOrigin, scripted: bool, event: &WindowEvent) -> bool {
    scripted && origin == input_trace::InputOrigin::User && is_input(event)
}

/// A person clicking into the window or away from it during a scripted run. A replay's look and
/// clicks act by the focus and the pointer grab, which that would change mid-run: the run keeps
/// the focus it started with, and the last change applies once it ends (`settle_after_run`).
fn held_focus(
    origin: input_trace::InputOrigin,
    scripted: bool,
    event: &WindowEvent,
) -> Option<bool> {
    match event {
        WindowEvent::Focused(focused) if scripted && origin == input_trace::InputOrigin::User => {
            Some(*focused)
        }
        _ => None,
    }
}

impl Game<'_> {
    /// winit's `window_event`. The control socket and the pad menu call it too, with the
    /// input they make up.
    pub(super) fn handle_window_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        event: WindowEvent,
        origin: input_trace::InputOrigin,
    ) {
        let input = matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::Ime(_)
        );
        self.with_user_input(input, |game| {
            game.handle_window_event_as(event_loop, id, event, origin);
        });
    }

    pub(super) fn handle_window_event_as(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        mut event: WindowEvent,
        origin: input_trace::InputOrigin,
    ) {
        // A headless game has no window: its events (the frame, the control socket's input)
        // carry `WindowId::dummy()`.
        let ours = match &self.window {
            Some(w) => w.id() == id,
            None => self.options.headless,
        };
        if !ours {
            return;
        }
        if locks_out(origin, self.scripted_run(), &event) {
            return;
        }
        if let Some(focused) = held_focus(origin, self.scripted_run(), &event) {
            self.focus_after_run = Some(focused);
            return;
        }
        follow_keymap(&mut event);
        // MacosUtil.setCtrlClickEmulatesRightClick: Control + left click is a right click.
        if let WindowEvent::MouseInput { state, button, .. } = &mut event
            && *button == MouseButton::Left
            && self.right_click_emulation.is_right_click(
                *state == ElementState::Pressed,
                self.modifiers.control_key(),
                cfg!(target_os = "macos") && self.settings.ctrl_click_emulates_right_click,
            )
        {
            *button = MouseButton::Right;
        }
        // Who caused it, before the event is handled: the trace wants the
        // window's own keys, never the ones the socket made up.
        self.trace_window_event(origin, &event);
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(key) = event.physical_key
        {
            if event.state == ElementState::Pressed {
                self.physical_keys.insert(key);
                horizon_client::keybinds::learn_layout(event);
            } else {
                self.physical_keys.remove(&key);
            }
        }
        if matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
        ) && !self.input_made_pointer_event()
        {
            self.pad.keyboard_took_over = true;
        }
        if let WindowEvent::MouseInput { state, button, .. } = &event {
            if *state == ElementState::Pressed {
                self.physical_buttons.insert(*button);
            } else {
                self.physical_buttons.remove(button);
            }
        }
        let mouse = if !self.pad_synthetic_input()
            && let WindowEvent::MouseInput { state, button, .. } = &event
        {
            Some((
                glfw_button(*button),
                *state == ElementState::Pressed,
                glfw_modifiers(self.modifiers),
            ))
        } else {
            None
        };
        if let Some((button, pressed, modifiers)) = mouse {
            let mut canceled = false;
            self.native_mouse_button_pre(button, pressed, modifiers, &mut canceled);
            if canceled {
                return;
            }
        }
        let resized = matches!(event, WindowEvent::Resized(_));
        self.handle_window_event_inner(event_loop, event);
        if resized {
            self.notify_window_resize();
        }
        // A title screen the event asked for opens before the next event.
        self.settle_title_mode();
        if let Some((button, pressed, modifiers)) = mouse {
            self.native_mouse_button_post(button, pressed, modifiers);
        }
    }

    /// One window event, in the order vanilla's `KeyboardHandler` and `MouseHandler` see it.
    ///
    /// A controller doesn't come through here in play: `pad_action` goes straight to the mappings
    /// (`mapped_key`, `mouse_mapping`). So a key handled below before `mapped_key_event` (a window
    /// shortcut, a special case) never reaches a controller. If a player could want it on one, give
    /// it a handler both can call and a `gamepad::Route` (screenshot and fullscreen do this). A
    /// screen's own raw keys, text and mods' key hooks stay raw on purpose.
    pub(super) fn handle_window_event_inner(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        mut event: WindowEvent,
    ) {
        let pressed = match &event {
            WindowEvent::KeyboardInput { event, .. } => {
                event.state == ElementState::Pressed && !event.repeat
            }
            WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
            _ => false,
        };
        if pressed && !self.pad.menu.synthetic {
            self.pad_radial_cancel();
        }
        if let WindowEvent::Focused(false) = &event {
            self.modifiers = winit::keyboard::ModifiersState::empty();
            self.physical_keys.clear();
            self.physical_buttons.clear();
            self.drag_button = None;
        }
        if let WindowEvent::ModifiersChanged(m) = &event {
            self.modifiers = m.state();
            horizon_client_input::modifiers::reconcile(
                self.modifiers,
                &mut self.physical_keys,
                &self.settings.keys,
                &mut self.toggles,
                [self.settings.toggle_crouch, self.settings.toggle_sprint],
                [
                    self.pad.holds_action("key.sneak"),
                    self.pad.holds_action("key.sprint"),
                ],
            );
            self.sync_toggle(SNEAK);
            self.sync_toggle(SPRINT);
        }
        // `PackSelectionScreen.onFilesDrop` on the Create New World screen.
        if let WindowEvent::DroppedFile(path) = &event
            && self.title_input_active()
            && let Some(title) = &mut self.title
        {
            title.drop_file(path.clone());
        }
        // GLFW never hands a field the keys an input method consumes: while a
        // composition is open, and the Enter or Escape that ends it (winit on
        // Windows still sends that one right after `Ime::Commit`).
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && key_event.state == ElementState::Pressed
            && self.text_field_focused()
            && horizon_client::preedit::should_suppress_key()
        {
            return;
        }
        // A character that comes with no key (Windows' Alt codes, `Alt` + digits on the numeric
        // keypad: a `WM_CHAR` after the Alt is released) reaches winit as a key event with no
        // physical key. GLFW's char callback types it in the focused text field (`charTyped`),
        // and every screen's key handler here would drop it for want of a key code.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && key_event.state == ElementState::Pressed
            && matches!(key_event.physical_key, PhysicalKey::Unidentified(_))
            && let Some(text) = key_event.text.as_deref()
        {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            if !text.is_empty() && self.text_field_focused() && self.types_text() {
                self.screen_text_each(&text, |game, c| {
                    // Err only says no box of the screen takes it.
                    let _ = game.type_chars_into_screen(c);
                });
            }
            return;
        }
        // Fullscreen is a window shortcut, before screens consume keyboard input.
        // Not Alt or Super on their own: leaving the window isn't playing with the keyboard.
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && !event.repeat
            && !matches!(
                event.physical_key,
                PhysicalKey::Code(
                    KeyCode::AltLeft | KeyCode::AltRight | KeyCode::SuperLeft | KeyCode::SuperRight
                )
            )
        {
            self.pad.keyboard_took_over = true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
            && horizon_client::keybinds::key_name(key)
                == Some(self.settings.keys.get("key.fullscreen"))
            && !(self.screen == Screen::Options && self.options_waiting_for_key())
        {
            self.last_input = Instant::now();
            if !event.repeat {
                self.toggle_fullscreen();
            }
            return;
        }
        // A screenshot is a window shortcut too: vanilla takes it before a
        // screen or a text box consumes the key (`handleGlobalKeyPress`), so
        // it works with chat, a sign or a book open.
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
        {
            self.last_input = Instant::now();
            if self.screenshot_shortcut(key, event.repeat) {
                return;
            }
        }
        // KeyboardHandler.keyPress: the debug keys come before the game's.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && let PhysicalKey::Code(key) = key_event.physical_key
        {
            use horizon_client::debug_keys::KeyAction;
            let action = match (key_event.state, key_event.repeat) {
                (ElementState::Released, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            if self.debug_key(key, action) {
                return;
            }
        }
        // KeyboardHandler.keyPress: the narrator hotkey, on every screen and
        // in game, before the screen takes the key.
        if let WindowEvent::KeyboardInput { event, .. } = &event {
            self.narrator_hotkey(event);
        }
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.last_input = Instant::now();
        }
        if !self.pad_synthetic_input()
            && matches!(
                event,
                WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. }
            )
        {
            self.reassert_pointer_capture();
        }
        // The real mouse takes the pointer back from the keyboard's focus.
        self.key_focus_mouse(event_loop, &event);
        // The screen's narration hears of the input before the screen takes
        // it; the pointer events the keyboard's focus sends aren't the mouse.
        if !self.key_focus_quiet() {
            narrator::narrate_input(&mut self.narration, &event, self.cursor);
        }
        // FileFixerProgressScreen: Cancel is the only way out (`shouldCloseOnEsc`
        // is false), so it takes the pointer and Escape does nothing.
        if let Some(starting) = &self.integrated_starting
            && starting.handle.upgrade().is_file_fixing()
        {
            let upgrade = starting.handle.upgrade();
            match &event {
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(r) = &self.renderer {
                        let b = HudBatch::new(
                            [r.config.width, r.config.height],
                            self.settings.gui_scale_inputs(),
                        );
                        self.cursor = (position.x as f32 / b.scale, position.y as f32 / b.scale);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if !upgrade.is_canceled() => {
                    let (w, h) = self.gui_size();
                    let (x, y) = horizon_client::level_loading::file_fixer_cancel(w, h);
                    if horizon_client::ui::hovered(self.cursor, x, y, 200.0, 20.0) {
                        self.audio.play_button_click();
                        upgrade.set_canceled();
                    }
                }
                _ => {}
            }
            if matches!(&event, WindowEvent::KeyboardInput { .. }) {
                return;
            }
        }
        if self.pending_entry.is_some()
            && matches!(&event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape))
        {
            self.pending_entry = None;
            self.perf.cancel_world_opening();
            return;
        }
        if (self.pending_entry.is_some() || self.integrated_starting.is_some())
            && !matches!(
                &event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return;
        }
        // The pack reload's progress overlay takes no input
        // (`LoadingOverlay`): `spawn_pack_reload` released what was held.
        if self.pack_reload.is_some()
            && !matches!(
                &event,
                WindowEvent::CloseRequested
                    | WindowEvent::Focused(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return;
        }
        if self.pending_save.is_some() {
            match event {
                WindowEvent::CloseRequested => {
                    if !self.settings.quit_shortcuts
                        && horizon_client::input::is_quit_shortcut_down(
                            self.modifiers,
                            &self.physical_keys,
                        )
                    {
                        return;
                    }
                    self.pending_save.as_mut().unwrap().quit = true;
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if !focused {
                        self.capture(false);
                    }
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Err(error) = self.render_closing_save() {
                        self.error = Some(error);
                        self.exit_on_error(event_loop);
                    }
                }
                _ => {}
            }
            return;
        }
        // ScreenEvent.KeyPressed.Pre: the open screen hears of the key before
        // it acts on it, and a cancelled press goes no further — no focus
        // move, no Escape-close, no inventory or hotbar key, no typing by the
        // key itself. It fires after the window's own
        // shortcuts (fullscreen, screenshot, the debug keys, the narrator) so
        // those keep working with a mod that cancels; the key's release still
        // comes through.
        //
        // A key's characters are a separate callback of GLFW's, and it comes
        // after the key callback (`KeyboardHandler.charTyped`): they are taken
        // off the key here and dispatched once the screen has acted on it, on
        // the screen that is open then (`ScreenEvent.CharacterTyped.Pre`).
        // Options keeps its text on the key: its own key path routes a key's
        // text between its boxes and its key-bind capture, and no key that
        // carries text closes that screen or moves its focus. The title's
        // screens keep theirs while no box has focus: Space presses a button
        // there, and the box that opens must not take its character.
        let mut typed: Option<String> = None;
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &mut event
            && key_event.state == ElementState::Pressed
        {
            let title_text = control::title_owns_text(&self.screen, self.title.is_some());
            if self.screen_hooks_open()
                && self.screen != Screen::Options
                && (!title_text || self.title.as_mut().is_some_and(|t| t.has_caret_field()))
            {
                typed = key_event.text.take().map(|text| text.to_string());
            }
            let canceled = match key_event.physical_key {
                PhysicalKey::Code(key) => self.screen_key_pressed(key, key_event.repeat),
                PhysicalKey::Unidentified(_) => false,
            };
            if self.screen_hooks_open()
                && (self.screen == Screen::Options || (title_text && self.types_text()))
            {
                // The hook after the key's own, then the text the key path
                // types in place (its own routing keeps it).
                if let Some(text) = key_event.text.as_deref() {
                    let left = self.screen_text_typed(text);
                    if left.as_deref() != Some(text) {
                        key_event.text = left.map(Into::into);
                    }
                }
            }
            if canceled {
                // charTyped is a separate callback: the characters still
                // reach the text box, without the key's own effects. Options'
                // characters already passed the hook in place.
                if let Some(text) = typed.as_deref() {
                    self.screen_chars_typed(Some(text));
                } else if self.screen == Screen::Options
                    && let Some(text) = key_event.text.as_deref()
                    && self.types_text()
                {
                    let text: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !text.is_empty() {
                        // Err only says no box of the screen takes it.
                        let _ = self.type_chars_into_screen(&text);
                    }
                }
                return;
            }
        }
        // Screen.keyPressed: Tab, the arrows and Enter move and press the
        // menu's focus, unless the focused widget or the screen keeps them.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && key_event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = key_event.physical_key
            && self.key_focus_key(event_loop, key)
        {
            // The keyboard's focus took the key; its characters are a separate
            // callback and still come (`KeyboardHandler.charTyped`).
            self.screen_chars_typed(typed.as_deref());
            return;
        }
        // The Post a mouse input owes once the screen's arms below have handled
        // it (`ScreenEvent.MouseButtonPressed.Post` and its siblings): set where
        // the Pre lets the input through, fired after the arms ran.
        let mut mouse_post: Option<native_hooks::ScreenMousePost> = None;
        let result: Result<()> = (|| {
            if let WindowEvent::MouseInput { state, button, .. } = &event
                && !self.pad_synthetic_input()
            {
                self.track_drag_button(*button, *state == ElementState::Pressed);
            }
            // The emote key let go over the wheel, a click on it.
            if self.screen == Screen::EmoteWheel && self.emote_wheel_event(&event) {
                return Ok(());
            }
            if let WindowEvent::KeyboardInput { event, .. } = &event
                && event.state == ElementState::Released
                && let PhysicalKey::Code(key) = event.physical_key
                && self.screen_key_released(key)
            {
                // Screen cancellation must not leave a held keyboard mapping down.
                self.keys.remove(&key);
                self.mapped_key_event(key, false, false)?;
                return Ok(());
            }
            if let WindowEvent::KeyboardInput { event, .. } = &event
                && event.state == ElementState::Released
                && self.screen == Screen::Options
                && let PhysicalKey::Code(key) = event.physical_key
            {
                self.ui.options.modifiers = self.modifiers().bits;
                let events = self.ui.options.key_released(&mut self.settings, key);
                self.handle_option_events(events)?;
            }
            // ScreenEvent.MouseButtonPressed.Pre / MouseButtonReleased.Pre:
            // the open screen hears of the press before anything reads it, and
            // a cancelled one goes no further — no hotbar key, no slot, no
            // button, no sort, no drag. First in the event, so none of the
            // arms below (which hold the container hotkey and the sort and
            // header buttons too) see it. A click the pad or the keyboard's
            // focus made up is not the mouse: it fires nothing.
            if let WindowEvent::MouseInput { state, button, .. } = &event
                && !self.pad_synthetic_input()
                && let Some(screen) = self.screen_view_now()
            {
                let (button, modifiers) = (glfw_button(*button), glfw_modifiers(self.modifiers));
                let mut canceled = false;
                if *state == ElementState::Pressed {
                    self.native_screen_mouse_button_pressed_pre(
                        &screen,
                        button,
                        modifiers,
                        &mut canceled,
                    );
                } else {
                    self.native_screen_mouse_button_released_pre(
                        &screen,
                        button,
                        modifiers,
                        &mut canceled,
                    );
                }
                if canceled {
                    return Ok(());
                }
                mouse_post = Some(native_hooks::ScreenMousePost::Button {
                    button,
                    pressed: *state == ElementState::Pressed,
                });
            }
            self.key_focus_mouse_press(&event);
            match event {
                WindowEvent::CloseRequested => {
                    if !self.settings.quit_shortcuts
                        && horizon_client::input::is_quit_shortcut_down(
                            self.modifiers,
                            &self.physical_keys,
                        )
                    {
                        return Ok(());
                    }
                    self.agent_cancel("the game window was closed; the game saves and exits");
                    self.begin_closing_save(true)?;
                }
                // Input method composition: the committed text types like a key's text.
                WindowEvent::Ime(winit::event::Ime::Commit(text)) => self.ime_commit(&text),
                WindowEvent::Ime(winit::event::Ime::Preedit(text, cursor)) => {
                    self.ime_preedit(&text, cursor)
                }
                WindowEvent::Ime(winit::event::Ime::Disabled) => {
                    horizon_client::preedit::set_preedit(None)
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                    self.observe_fullscreen();
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    self.focus_lost = (!focused).then(Instant::now);
                    if focused {
                        self.regrab_on_refocus();
                    } else {
                        // A held pointer goes with the focus, wherever the
                        // system keeps the grab: the window asks for it back
                        // when the focus returns (`regrab_on_refocus`).
                        let held = self.captured;
                        if self.control.is_none() {
                            self.keys.clear();
                            // Latched sneak/sprint live in `toggles`, not in the
                            // physical keys: put them back so a short alt-tab
                            // (no pause screen) does not drop them. Only a mapping
                            // whose toggle option is on is latched; a held one has
                            // no state to keep, and an unfocused window never gets
                            // its key release, so leaving it down would keep the
                            // player sneaking or sprinting until the key is tapped.
                            for (slot, needs_toggle) in [
                                self.settings.toggle_crouch,
                                self.settings.toggle_sprint,
                                self.settings.toggle_attack,
                                self.settings.toggle_use,
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                self.toggles.0[slot].set_down(false, needs_toggle);
                            }
                            self.sync_toggle(0);
                            self.sync_toggle(1);
                            self.left = false;
                            self.right = false;
                            self.capture(false);
                        }
                        self.regrab_on_refocus = held;
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(r) = &self.renderer {
                        let b = HudBatch::new(
                            [r.config.width, r.config.height],
                            self.settings.gui_scale_inputs(),
                        );
                        let previous = self.cursor;
                        self.cursor = (position.x as f32 / b.scale, position.y as f32 / b.scale);
                        if self.studio_mouse_dragged(b.width) {
                            return Ok(());
                        }
                        match self.screen_mouse_dragged(previous) {
                            native_hooks::ScreenMousePre::Canceled => return Ok(()),
                            native_hooks::ScreenMousePre::Passed(post) => mouse_post = Some(post),
                            native_hooks::ScreenMousePre::NotFired => {}
                        }
                        if self.dialog_open() {
                            self.dialog_drag();
                        }
                        if self.screen == Screen::BlockEditor {
                            self.block_editor_drag();
                        }
                        if self.screen == Screen::Advancements {
                            self.advancements_drag();
                        }
                        if self.screen == Screen::Stats {
                            self.stats_drag();
                        }
                        if self.screen == Screen::DebugOptions {
                            self.debug_options_drag();
                        }
                        if self.screen == Screen::HudEditor {
                            self.hud_editor_drag();
                        }
                        if self.screen == Screen::WorldOptions {
                            self.world_options_drag();
                        }
                        if self.screen == Screen::Creative {
                            let (_, oy) = Self::creative_origin(&b);
                            self.creative_screen.drag_scrollbar(self.cursor.1 - oy);
                        }
                        if self.screen == Screen::Inventory {
                            self.station_drag();
                            self.recipe_book_drag();
                        }
                        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
                            self.inventory_drag_move();
                        }
                        if self.title_input_active()
                            && let Some(title) = &mut self.title
                        {
                            title.drag(self.cursor, (b.width, b.height));
                            title.drag_field(&self.gui.atlas, self.cursor.0);
                        }
                        if self.dragging && self.screen == Screen::Options {
                            let _ = b;
                            self.options_drag()?;
                        }
                    }
                }
                WindowEvent::MouseWheel { .. }
                    if self.screen == Screen::Playing && self.replay.driver.is_some() => {}
                WindowEvent::MouseWheel { delta, .. } => {
                    let (raw_x, raw_y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                        MouseScrollDelta::PixelDelta(p) => (p.x / 30.0, p.y / 30.0),
                    };
                    // MouseHandler.onScroll: discrete scrolling and the wheel sensitivity.
                    let discrete = self.settings.discrete_scroll;
                    let wheel_sensitivity = f64::from(self.settings.mouse_wheel_sensitivity);
                    let scaled = |v: f64| {
                        (if discrete {
                            horizon_client::mouse::java_signum(v)
                        } else {
                            v
                        }) * wheel_sensitivity
                    };
                    let d = scaled(raw_y) as f32;
                    // ScreenEvent.MouseScrolled.Pre: the open screen hears of
                    // the wheel before anything scrolls, and a cancelled turn
                    // scrolls nothing — no bundle, no list, slider or
                    // scrollbar. The hotbar below reads the wheel only while
                    // playing, which the gate excludes. A wheel the pad or the
                    // keyboard's focus made up is not the mouse: it fires
                    // nothing.
                    if !self.pad_synthetic_input()
                        && let Some(screen) = self.screen_view_now()
                    {
                        let mut canceled = false;
                        self.native_screen_mouse_scrolled_pre(
                            &screen,
                            [scaled(raw_x), scaled(raw_y)],
                            glfw_modifiers(self.modifiers),
                            &mut canceled,
                        );
                        if canceled {
                            return Ok(());
                        }
                        mouse_post = Some(native_hooks::ScreenMousePost::Scroll([
                            scaled(raw_x),
                            scaled(raw_y),
                        ]));
                    }
                    // A prompt is vanilla's `Minecraft.screen`, the only one MouseHandler.onScroll
                    // tells: the top one hears the wheel, as the click arms below pick it, and
                    // nothing behind it does.
                    if self.prompt_open() {
                        if self.command_prompt.is_some() || self.link_prompt.is_some() {
                        } else if self.code_of_conduct.is_some() {
                            self.code_of_conduct_scroll(d);
                        } else if self.server_packs.prompt.is_none() {
                            self.dialog_scroll(d);
                        }
                        return Ok(());
                    }
                    let stick_scroll = self.pad_stick_scroll();
                    let step_scroll = self.pad_step_scroll();
                    if self.title_input_active()
                        && self.screen != Screen::Win
                        && let Some(title) = &mut self.title
                    {
                        let size = self
                            .window
                            .as_ref()
                            .map(|w| w.inner_size())
                            .unwrap_or_default();
                        let batch = HudBatch::new(
                            [size.width, size.height],
                            self.settings.gui_scale_inputs(),
                        );
                        let gui = (batch.width, batch.height);
                        if step_scroll {
                            title.step_scroll(d, self.cursor, gui);
                        } else if stick_scroll {
                            title.stick_scroll(d, self.cursor, gui);
                        } else {
                            title.scroll_worlds(d, self.cursor, gui);
                        }
                    }
                    // MouseHandler.onScroll: the hotbar (or the spectator
                    // menu) takes the wheel only when no screen is open; a
                    // book, lectern, sign or prompt drawn over gameplay is one.
                    if self.hotbar_takes_wheel() {
                        let scroll =
                            self.mouse
                                .on_scroll(raw_x, raw_y, discrete, wheel_sensitivity);
                        let (horizontal_steps, vertical_steps) =
                            scroll.map_or((0, 0), |(_, steps)| steps);
                        if self.hotbar_scroll_canceled(
                            [scaled(raw_x), scaled(raw_y)],
                            [horizontal_steps, vertical_steps],
                        ) {
                            return Ok(());
                        }
                        let Some((wheel, _)) = scroll else {
                            return Ok(());
                        };
                        self.hotbar_wheel_step(wheel, vertical_steps);
                    }
                    if self.screen == Screen::Chat && d != 0.0 {
                        self.chat_scroll(d);
                    }
                    if self.screen == Screen::Social && d != 0.0 {
                        self.social_scroll(d);
                    }
                    if self.screen == Screen::CommandBlock && d != 0.0 {
                        self.command_block_scroll(d);
                    }
                    if self.screen == Screen::Options && d != 0.0 {
                        self.options_scroll(d)?;
                    }
                    // AbstractContainerScreen.mouseScrolled: a bundle under the
                    // cursor takes the wheel before the screen's own scrolling.
                    let bundle = bundles::takes_bundle_wheel(self.screen)
                        && d != 0.0
                        && self.bundle_scroll(scaled(raw_x), scaled(raw_y));
                    if self.screen == Screen::Creative && d != 0.0 && !bundle {
                        self.creative_screen.wheel(f64::from(d), &self.pack.names);
                    }
                    if self.screen == Screen::Inventory
                        && d != 0.0
                        && !bundle
                        && !self.tweak_scroll(d, self.inventory_target().0)
                    {
                        self.station_scroll(d);
                    }
                    if self.screen == Screen::Stats && d != 0.0 {
                        self.stats_scroll(d);
                    }
                    if self.screen == Screen::WorldOptions && d != 0.0 {
                        self.world_options_scroll(d);
                    }
                    if self.screen == Screen::ServerLinks && d != 0.0 {
                        self.server_links_scroll(d);
                    }
                    if self.screen == Screen::DebugOptions && d != 0.0 {
                        self.debug_options_scroll(d);
                    }
                    if self.screen == Screen::HudEditor && d != 0.0 {
                        self.hud_editor_scroll(f64::from(d));
                    }
                    if self.screen == Screen::Advancements {
                        self.advancements_scroll(scaled(raw_x) as f32, d);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.command_prompt.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.command_prompt_click();
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.link_prompt.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.link_prompt_click();
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.code_of_conduct.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.code_of_conduct_click()?;
                    }
                }
                WindowEvent::MouseInput { state, button, .. }
                    if self.server_packs.prompt.is_some() =>
                {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.pack_prompt_click()?;
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.dialog_open() => {
                    if button == MouseButton::Left {
                        if state == ElementState::Pressed {
                            self.dialog_click()?;
                        } else {
                            self.dialog_release();
                        }
                    }
                }
                // ConnectScreen's Cancel, under the prompts and dialogs above.
                WindowEvent::MouseInput { state, button, .. } if self.remote_loading() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.connect_screen_click()?;
                    }
                }
                WindowEvent::MouseInput { .. } if self.screen == Screen::Win => {}
                WindowEvent::MouseInput { state, button, .. } if self.screen == Screen::Options => {
                    if self.options_waiting_for_key() && state == ElementState::Pressed {
                        self.options_bind_mouse(button)?;
                    } else if matches!(button, MouseButton::Left | MouseButton::Right) {
                        self.options_mouse(
                            state == ElementState::Pressed,
                            button == MouseButton::Right,
                        )?;
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } if self.title_input_active() && self.screen != Screen::Options => {
                    if let Some(title) = &mut self.title {
                        title.release();
                        title.release_field();
                    }
                }
                WindowEvent::MouseInput {
                    state,
                    button: MouseButton::Left,
                    ..
                } if self.title_input_active() && state == ElementState::Pressed => {
                    let size = self
                        .renderer
                        .as_ref()
                        .map(|r| [r.config.width, r.config.height])
                        .unwrap_or([1, 1]);
                    let b = HudBatch::new(size, self.settings.gui_scale_inputs());
                    let atlas = &self.gui.atlas;
                    let shift = self.has_shift_down();
                    let (action, sound) = self.title.as_mut().map_or((Action::None, false), |t| {
                        let clicked = t.click(self.cursor.0, self.cursor.1, (b.width, b.height));
                        t.press_field(atlas, self.cursor, (b.width, b.height), shift);
                        let rules = t.press_rules(atlas, self.cursor, (b.width, b.height));
                        (clicked.0, clicked.1 || rules)
                    });
                    self.save_title_options();
                    if sound {
                        self.audio.play_button_click();
                    }
                    self.title_action(event_loop, action)?;
                    self.reopen_data_packs(event_loop)?;
                }
                // AbstractSelectionList.mouseClicked: the world list takes a
                // click of any button, so the middle one selects its row
                // without joining (only the left click's play icon or its
                // click on the selected row does that).
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Middle,
                    ..
                } if self.title_input_active() && self.screen != Screen::Options => {
                    let size = self
                        .renderer
                        .as_ref()
                        .map(|r| [r.config.width, r.config.height])
                        .unwrap_or([1, 1]);
                    let b = HudBatch::new(size, self.settings.gui_scale_inputs());
                    if let Some(title) = self.title.as_mut() {
                        title.select_world_at(self.cursor.0, self.cursor.1, (b.width, b.height));
                    }
                }
                WindowEvent::MouseInput { .. } if self.title_input_active() => {}
                WindowEvent::MouseInput { state, button, .. } if self.book.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        let size = self
                            .renderer
                            .as_ref()
                            .map(|r| [r.config.width, r.config.height])
                            .unwrap_or([1, 1]);
                        self.book_click(size);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.sign_edit.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        let size = self
                            .renderer
                            .as_ref()
                            .map(|r| [r.config.width, r.config.height])
                            .unwrap_or([1, 1]);
                        self.sign_edit_click(size);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    let pressed = state == ElementState::Pressed;
                    if self.studio_mouse_input(pressed, button) {
                        return Ok(());
                    }
                    if matches!(self.screen, Screen::Inventory | Screen::Creative)
                        && let Some(key) = self
                            .settings
                            .keys
                            .container_mouse_key(button, self.creative)
                    {
                        if pressed && self.carried.is_none() {
                            self.inventory_hotkey(key);
                        }
                        // Neither edge of a bound button is an ordinary slot click.
                        return Ok(());
                    }
                    if pressed
                        && (self.sort_mouse_press(button)
                            || self.hud_editor_key_press(&horizon_client::keybinds::mouse_name(
                                button,
                            )))
                    {
                        return Ok(());
                    }
                    match self.screen {
                        Screen::Playing => {
                            if pressed && !self.captured {
                                self.capture(true);
                                return Ok(());
                            }
                            let input = horizon_client::keybinds::mouse_name(button);
                            if self.key_mapping_canceled(Some(input.as_str()), pressed) {
                                self.physical_buttons.remove(&button);
                                return Ok(());
                            }
                            self.extras_mouse(button, pressed);
                            if pressed
                                && self.settings.keys.get("key.smoothCamera")
                                    == horizon_client::keybinds::mouse_name(button)
                            {
                                self.smooth_camera_action();
                            }
                            for mapped in self.settings.keys.translate_mouse_all(button) {
                                match mapped {
                                    Err(b)
                                        if !self.shared_default_acts(b, Some(input.as_str())) => {}
                                    Err(b) => self.mouse_mapping(b, pressed, Source::Keyboard)?,
                                    Ok(k) if pressed => {
                                        self.keys.insert(k);
                                        self.key_pressed(k, k)?;
                                    }
                                    Ok(k) => {
                                        self.keys.remove(&k);
                                    }
                                }
                            }
                        }
                        Screen::Inventory | Screen::Creative
                            if !matches!(button, MouseButton::Left | MouseButton::Right) =>
                        {
                            // AbstractContainerScreen: another button clicks
                            // a slot only as the pick block key (its hotbar
                            // keys were handled above).
                            if self.settings.keys.get("key.pickItem")
                                == horizon_client::keybinds::mouse_name(button)
                            {
                                if pressed {
                                    self.inventory_pick_press();
                                } else {
                                    self.inventory_pick_release();
                                }
                                if let Some(key) = self.container {
                                    self.container_changed(key);
                                }
                            }
                        }
                        Screen::Inventory if pressed => {
                            self.inventory_press(button == MouseButton::Right);
                            if let Some(key) = self.container {
                                self.container_changed(key);
                            }
                        }
                        Screen::Inventory => {
                            self.inventory_mouse_release(button == MouseButton::Right);
                            self.station_release();
                            self.recipe_book_release();
                        }
                        Screen::Chat if pressed && button == MouseButton::Left => self.chat_click(),
                        Screen::Creative if pressed => {
                            self.click_creative(button == MouseButton::Right);
                        }
                        Screen::Creative => {
                            if button == MouseButton::Left {
                                self.creative_screen.release();
                            }
                            self.inventory_mouse_release(button == MouseButton::Right);
                        }
                        Screen::Paused
                            if pressed && button == MouseButton::Left && self.pause_menu =>
                        {
                            self.pause_click()?
                        }
                        Screen::WorldOptions if pressed && button == MouseButton::Left => {
                            self.world_options_click()
                        }
                        Screen::ServerLinks if pressed && button == MouseButton::Left => {
                            self.server_links_click()?
                        }
                        Screen::DebugOptions if pressed && button == MouseButton::Left => {
                            self.debug_options_click()
                        }
                        Screen::DebugOptions if button == MouseButton::Left => {
                            self.debug_options_release()
                        }
                        Screen::HudEditor
                            if pressed
                                && matches!(button, MouseButton::Left | MouseButton::Right) =>
                        {
                            self.hud_editor_press(button == MouseButton::Right)
                        }
                        Screen::HudEditor if button == MouseButton::Left => {
                            self.hud_editor_release()
                        }
                        Screen::ConfirmLink if pressed && button == MouseButton::Left => {
                            self.confirm_link_click()
                        }
                        Screen::DialogWarning if pressed && button == MouseButton::Left => {
                            self.dialog_warning_click()?
                        }
                        Screen::WorldOptions if button == MouseButton::Left => {
                            self.world_options_release()
                        }
                        Screen::Friends if pressed && button == MouseButton::Left => {
                            self.friends_screen_click()
                        }
                        Screen::Social if pressed && button == MouseButton::Left => {
                            self.social_click()
                        }
                        Screen::CommandBlock if pressed && button == MouseButton::Left => {
                            self.command_block_click()
                        }
                        Screen::BlockEditor if pressed && button == MouseButton::Left => {
                            self.block_editor_click()
                        }
                        Screen::BlockEditor if button == MouseButton::Left => {
                            self.block_editor_release()
                        }
                        Screen::Dead if pressed && button == MouseButton::Left => {
                            self.death_click()?
                        }
                        Screen::Advancements if button == MouseButton::Left => {
                            let size = self
                                .renderer
                                .as_ref()
                                .map_or([1, 1], |r| [r.config.width, r.config.height]);
                            self.advancements_mouse(pressed, size);
                        }
                        Screen::Stats if pressed && button == MouseButton::Left => {
                            let size = self
                                .renderer
                                .as_ref()
                                .map_or([1, 1], |r| [r.config.width, r.config.height]);
                            self.stats_click(size);
                        }
                        Screen::Stats if button == MouseButton::Left => self.stats_release(),
                        Screen::GameModeSwitcher if !pressed => self.switcher_release(button),
                        _ => {}
                    }
                }
                // ConfirmScreen.keyPressed: Escape answers no, once its delay is over.
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Dead && self.ui.death_confirm.is_some() =>
                {
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                        && self
                            .ui
                            .death_confirm
                            .is_some_and(|t| t.elapsed().as_millis() >= 1000)
                    {
                        self.death_confirm_answer(false)?;
                    }
                }
                // ConfirmScreen.keyPressed: Escape answers no.
                WindowEvent::KeyboardInput { event, .. } if self.command_prompt.is_some() => {
                    if event.state == ElementState::Pressed
                        && let PhysicalKey::Code(key) = event.physical_key
                    {
                        self.command_prompt_key(key);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.link_prompt.is_some() => {
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    {
                        self.answer_link_prompt(false);
                    }
                }
                // CodeOfConductScreen.shouldCloseOnEsc is false: no key leaves the screen.
                WindowEvent::KeyboardInput { .. } if self.code_of_conduct.is_some() => {}
                WindowEvent::KeyboardInput { event, .. } if self.server_packs.prompt.is_some() => {
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    {
                        self.answer_pack_prompt(false)?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.dialog_open() => {
                    let shortcut = self.edit_shortcut(&event);
                    if event.state == ElementState::Pressed
                        && let PhysicalKey::Code(key) = event.physical_key
                    {
                        self.dialog_key(key, shortcut, event.text.as_deref())?
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Chat && self.title.is_none() =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        if event.state == ElementState::Pressed {
                            self.keys.insert(key);
                            self.chat_key(&event);
                        } else {
                            self.keys.remove(&key);
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.sign_edit.is_some() => {
                    self.sign_edit_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.book.is_some() => {
                    self.book_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Creative
                        && self.creative_screen.tab().search
                        && !self.mod_browser_typing()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key
                        && self.search_tab_hotbar_key(key)
                    {
                        // The hovered slot took the key, and its characters
                        // go nowhere (`ignoreTextInput`).
                        typed = None;
                    } else if let Some(shortcut) = self.edit_shortcut(&event) {
                        self.creative_search_shortcut(shortcut);
                    } else if let PhysicalKey::Code(key) = event.physical_key {
                        // EditBox.keyPressed: arrows, Home, End, Delete and Backspace.
                        let before = self.creative_screen.search.value.clone();
                        let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                        if self.creative_screen.search.key(key, ctrl, shift) {
                            if self.creative_screen.search.value != before {
                                self.creative_screen.scroll = 0.0;
                            }
                        } else {
                            self.creative_search_edit(event.text.as_deref());
                        }
                    } else {
                        self.creative_search_edit(event.text.as_deref());
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.recipe_search_focused()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // The recipe book's search box takes every key but Escape.
                    self.recipe_key(&event);
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.mod_browser_typing()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // A mod's item panel's search box, as the recipe book's.
                    self.mod_browser_key(&event);
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.station_typing()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // The anvil rename box takes every key but Escape (AnvilScreen.keyPressed).
                    if let Some(shortcut) = self.edit_shortcut(&event) {
                        self.station_shortcut(shortcut);
                    } else if let PhysicalKey::Code(key) = event.physical_key
                        && self.station_key(key)
                    {
                    } else {
                        let backspace = event.physical_key == PhysicalKey::Code(KeyCode::Backspace);
                        self.station_text(event.text.as_deref(), backspace);
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::WorldOptions
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    self.world_options_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::Win => {
                    self.win_key(&event)?
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen.takes_presses() && event.state == ElementState::Pressed =>
                {
                    self.screen_press(&event)
                }
                // Options consumes presses; releases still reach the bound
                // gameplay mapping below, including sneak/sprint toggles.
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Options && event.state == ElementState::Pressed =>
                {
                    if let Some(shortcut) = self.edit_shortcut(&event)
                        && self.options_shortcut(shortcut)
                    {
                        return Ok(());
                    }
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.options_key(key, event.text.as_deref())?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Paused
                        && event.state == ElementState::Pressed
                        && self.pause_menu
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.native_panel_key_event(key, true, event.repeat);
                    }
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.pause_key(key)?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Dead && event.state == ElementState::Pressed =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.native_panel_key_event(key, true, event.repeat);
                    }
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.death_key(key)?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.title_input_active() => {
                    if event.state == ElementState::Pressed {
                        if let Some(shortcut) = self.edit_shortcut(&event) {
                            self.title_shortcut(shortcut);
                            return Ok(());
                        }
                        // `TabNavigationBar.keyPressed` comes before the open
                        // tab's own widgets: Ctrl plus a digit or Tab picks a
                        // tab instead of reaching them.
                        if let PhysicalKey::Code(key) = event.physical_key {
                            let ctrl = self.has_control_down_with_quirk();
                            let shift = self.has_shift_down();
                            let tab = self
                                .title
                                .as_mut()
                                .and_then(|t| t.tab_bar_key(key, ctrl, shift));
                            if let Some(sound) = tab {
                                if sound {
                                    self.audio.play_button_click();
                                }
                                return Ok(());
                            }
                        }
                        let (w, h) = self.gui_size();
                        match event.physical_key {
                            // `WorldListEntry.keyPressed`: Space joins too, with the button click sound.
                            PhysicalKey::Code(
                                code @ (KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space),
                            ) if code != KeyCode::Space
                                || self.title.as_mut().is_some_and(|t| {
                                    !t.has_caret_field()
                                        && (t.list_focused() || t.focused_button.is_some())
                                }) =>
                            {
                                let (action, sound) = self
                                    .title
                                    .as_mut()
                                    .map_or((Action::None, false), |t| t.confirm_key_size((w, h)));
                                self.save_title_options();
                                if sound {
                                    self.audio.play_button_click();
                                }
                                self.title_action(event_loop, action)?;
                            }
                            PhysicalKey::Code(
                                key @ (KeyCode::Backspace
                                | KeyCode::Delete
                                | KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::Home
                                | KeyCode::End),
                            ) if self.title.as_mut().is_some_and(|t| t.has_caret_field()) => {
                                let (ctrl, shift) =
                                    (self.has_control_down(), self.has_shift_down());
                                if let Some(t) = self.title.as_mut() {
                                    t.field_key(key, ctrl, shift);
                                }
                            }
                            PhysicalKey::Code(
                                key @ (KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::ArrowUp
                                | KeyCode::ArrowDown),
                            ) => {
                                let shift = self.has_shift_down();
                                if let Some(t) = self.title.as_mut() {
                                    t.arrow_key(key, (w, h), shift);
                                }
                            }
                            PhysicalKey::Code(KeyCode::Backspace) => self.title_edit(None, true),
                            PhysicalKey::Code(KeyCode::Tab) => {
                                let shift = self.has_shift_down();
                                if let Some(t) = self.title.as_mut() {
                                    t.tab_dir(shift, (w, h));
                                }
                            }
                            PhysicalKey::Code(KeyCode::Escape) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.escape();
                                    self.reopen_data_packs(event_loop)?;
                                }
                                self.save_title_options();
                            }
                            PhysicalKey::Code(KeyCode::F5) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.refresh_servers();
                                }
                            }
                            _ => self.title_edit(event.text.as_deref(), false),
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    let PhysicalKey::Code(key) = event.physical_key else {
                        return Ok(());
                    };
                    let pressed = event.state == ElementState::Pressed;
                    if self.key_press_canceled(key, pressed, event.repeat) {
                        return Ok(());
                    }
                    self.native_panel_key_event(
                        key,
                        event.state == ElementState::Pressed,
                        event.repeat,
                    );
                    // Screen.keyPressed sees the physical key, repeats included.
                    if event.state == ElementState::Pressed
                        && self.screen == Screen::Inventory
                        && (self.merchant_key(key) || self.recipe_book_key(key))
                    {
                        return Ok(());
                    }
                    if event.state == ElementState::Pressed
                        && !event.repeat
                        && horizon_client::keybinds::key_name(key)
                            .is_some_and(|n| self.settings.keys.get("key.smoothCamera") == n)
                    {
                        self.smooth_camera_action();
                    }
                    // ScreenEvent.KeyPressed.Post / KeyReleased.Post: no screen arm
                    // took the key, so the open screen's `keyPressed` returned
                    // false; a cancel marks it handled and the game's own key
                    // handling never sees it.
                    if self.screen_key_post(key, event.state == ElementState::Pressed, event.repeat)
                    {
                        return Ok(());
                    }
                    self.mapped_key_event(key, event.state == ElementState::Pressed, event.repeat)?;
                }
                WindowEvent::RedrawRequested => {
                    horizon_client::spans::time("recover_device", || self.recover_device())?;
                    self.observe_fullscreen();
                    self.sync_ime();
                    let now = Instant::now();
                    let frame_cpu = self
                        .options
                        .spikes
                        .and_then(|_| horizon_client::spans::thread_cpu());
                    let elapsed = now.duration_since(self.previous);
                    let between = now.saturating_duration_since(self.frame_end);
                    self.frame_ticks = elapsed.as_secs_f32() * 20.0;
                    self.previous = now;
                    self.studio_fly(elapsed);
                    // Arrow-key camera controls belong to gameplay; chat and
                    // other screens use the same keys for their own navigation.
                    if self.screen == Screen::Playing
                        && self.title.is_none()
                        && self.sign_edit.is_none()
                        && self.book.is_none()
                        && !self.dialog_open()
                    {
                        let turn = |k| f32::from(u8::from(self.keys.raw(&k)));
                        let secs = elapsed.as_secs_f32().min(0.1);
                        let yaw = (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0;
                        let pitch = (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0;
                        if yaw != 0.0 || pitch != 0.0 {
                            // 90 degrees a second, as `Entity.turn` degrees.
                            self.turn_player(
                                f64::from(yaw * secs / 0.15),
                                f64::from(pitch * secs / 0.15),
                            );
                        }
                    }
                    if self.title.is_none() {
                        horizon_client::spans::time("update_world", || self.update_world());
                        horizon_client::spans::time("spawn_player", || self.spawn_player())?;
                    }
                    let updated = Instant::now();
                    self.track_toggle_screens();
                    self.heal_hold_latches();
                    horizon_hooks::engine::published(self.is_published());
                    self.report_activity();
                    let target = self.tick_target_millis();
                    let steps = self.clock.advance(elapsed, target);
                    for _ in 0..steps {
                        let started = Instant::now();
                        horizon_client::social::set_presence_status(self.presence_status());
                        horizon_client::social::tick();
                        // Minecraft.tick: every client tick, paused or not, in menus too.
                        self.native_client_tick_pre();
                        if self.title.is_none() && !self.pause {
                            if self.tick_script.armed() {
                                self.tick_script_step(event_loop);
                            }
                            horizon_client::spans::time("tick", || self.tick())?;
                            // Cave mood (a shader pack's playerMood) samples on a random
                            // seeded from the clock: under `HORIZON_STILL` it waits for the
                            // world and draws from its tick.
                            if !self.clock.still() {
                                self.tick_ambience();
                            } else if self.clock.runs_normally() {
                                self.ambience.reseed(self.visual_ticks() as i64);
                                self.tick_ambience();
                            }
                            self.tick_fov();
                        }
                        self.tick_win_screen()?;
                        self.native_configs_tick_client();
                        // Gui.tick runs ChatListener.tick paused or not.
                        self.tick_chat_queue();
                        self.tick_music();
                        self.tick_sound_engine();
                        // ClientTickEvent.Post, the tick's work done.
                        self.native_client_tick_post();
                        self.clock.record(started.elapsed());
                        if let Some(report) = self.clock.take_sprint_report() {
                            self.sprint_report(&report);
                            self.on_tick_rate_changed();
                        }
                    }
                    self.extras_frame(elapsed);
                    self.extras_retry()?;
                    self.apply_mouse_look();
                    self.input_look(elapsed.as_secs_f32());
                    self.advance_flight();
                    self.tick_world_icon();
                    let ticked = Instant::now();
                    let rendered = self.render_and_flip(elapsed)?;
                    horizon_client::spans::time("pr_cursor", || self.apply_cursor_icon());
                    // The frame's spans, taken every frame they are recorded so they never
                    // pile up: the slow-frame line's.
                    let frame_spans = if horizon_client::spans::recording_here() {
                        horizon_client::spans::take_raw()
                    } else {
                        Vec::new()
                    };
                    if horizon_client::spans::trace::due() {
                        horizon_client::spans::trace::stop();
                    }
                    if let Some(threshold) = self.options.spikes
                        && let Some(r) = self.renderer.as_ref()
                    {
                        let cpu = now.elapsed().saturating_sub(r.last_wait).as_secs_f32() * 1000.0;
                        // Spans timed between frames (`idle`) come first.
                        let spans = horizon_client::spans::merge(&frame_spans);
                        let between = between.as_secs_f32() * 1000.0;
                        if cpu > threshold || between > threshold {
                            // The render thread's own CPU time in the frame:
                            // far below the wall time, the frame waited.
                            let worked = horizon_client::spans::thread_cpu()
                                .zip(frame_cpu)
                                .map_or_else(String::new, |(end, start)| {
                                    format!(", cpu {:.1} ms", (end - start).as_secs_f32() * 1000.0)
                                });
                            let gap = if between > threshold {
                                format!(", {between:.1} ms before it")
                            } else {
                                String::new()
                            };
                            log::info!(
                                "Horizon: slow frame {cpu:.1} ms{worked}{gap} ({steps} ticks): {}",
                                horizon_client::spans::describe(&spans)
                            );
                        }
                    }
                    if let Some(r) = self.renderer.as_mut() {
                        let cpu = now.elapsed().saturating_sub(r.last_wait);
                        if rendered {
                            let ms = |d: Duration| d.as_secs_f32() * 1000.0;
                            self.frame_graph.record(
                                ms(elapsed),
                                ms(cpu),
                                r.gpu_times.last().copied(),
                            );
                            if let Some(&gpu) = r.gpu_times.last() {
                                self.gpu_utilization =
                                    f64::from(gpu) * 100.0 / f64::from(ms(cpu).max(0.001));
                            }
                            if self.settings.horizon.gpu_culling
                                == horizon_client::graphics::GpuCullingMode::Auto
                                && self.screen == Screen::Playing
                                && !self.streaming.terrain_loading()
                                && horizon_client::graphics::gpu_culling_offer()
                                    .reason()
                                    .is_none()
                                && let Some(on) = self.gpu_culling_auto.observe(
                                    ms(cpu),
                                    r.gpu_times.last().copied(),
                                    ms(elapsed),
                                )
                            {
                                log::info!(
                                    "Horizon: GPU Culling Auto turns it {} ({})",
                                    if on { "on" } else { "off" },
                                    self.gpu_culling_auto.state()
                                );
                                if r.set_gpu_culling(on).is_err() {
                                    self.gpu_culling_auto.reset();
                                }
                            }
                            self.frame_stats.record_cpu(ms(cpu));
                            let render = r.last_encode;
                            self.frame_stats.record_phases([
                                ms(updated - now),
                                ms(ticked - updated),
                                ms(ticked.elapsed().saturating_sub(r.last_wait + render)),
                                ms(render),
                            ]);
                            self.frame_stats
                                .record_block_entities(r.static_block_entity_frame_stats());
                        }
                        self.frame_stats.record_gpu(&r.gpu_times);
                        r.gpu_times.clear();
                    }
                    self.limit_frame();
                    let paused = self.pauses_game();
                    if paused != self.pause {
                        let mut cancel = false;
                        self.native_client_pause_change_pre(paused, &mut cancel);
                        if !cancel {
                            self.pause = paused;
                            self.native_client_pause_change_post(paused);
                        }
                    }
                    if let Some(server) = &self.integrated {
                        server.set_paused(self.pause);
                        server.set_distances(
                            self.settings.render_distance,
                            self.settings.simulation_distance,
                            self.settings.entity_distance,
                        );
                        server.set_legacy_rules(self.settings.extras.pvp.combat.rules);
                    }
                    self.clock.end_frame(self.pause);
                    self.poll_screenshot_writes();
                    let mut consumed = !self.ui.screenshot_messages.is_empty();
                    let finished = self
                        .renderer
                        .as_mut()
                        .and_then(|r| r.screenshot_result.take())
                        .and_then(|result| match result {
                            Ok(pending) => self.save_screenshot(pending),
                            Err(error) => {
                                let result = Err(error);
                                self.finish_screenshot(&result, None);
                                Some(result)
                            }
                        });
                    if let Some(result) = &finished
                        && let Some(reply) = self.pending_screenshot.take()
                    {
                        let _ = reply.send(
                            match result {
                                Ok([w, h]) => {
                                    serde_json::json!({ "ok": true, "width": w, "height": h })
                                }
                                Err(e) => serde_json::json!({ "error": e }),
                            }
                            .to_string(),
                        );
                        consumed = true;
                    }
                    if let Some(result) =
                        self.renderer.as_mut().and_then(|r| r.capture_result.take())
                        && let Some(reply) = self.pending_render_capture.take()
                    {
                        let value = result.unwrap_or_else(|e| serde_json::json!({ "error": e }));
                        let _ = reply.send(value.to_string());
                    }
                    // A result nobody asked for was the world icon's
                    // (`tick_world_icon`): a failure tries again a second
                    // later, as `tryTakeScreenshotIfNeeded` does.
                    if !consumed && let Some(Err(error)) = finished {
                        log::warn!("Horizon: couldn't save the world icon: {error}");
                    }
                    if rendered {
                        self.frame_stats.record(elapsed.as_secs_f32() * 1000.0);
                        self.debug_charts.frame_times.log(elapsed.as_nanos() as i64);
                        if self.frames == 0 {
                            // The launcher times Play to this line (launcher/src/perf.rs).
                            log::info!(
                                "Horizon: first frame after {} ms",
                                self.started.elapsed().as_millis()
                            );
                        }
                        if self.title.is_some() {
                            self.perf.title_shown();
                        }
                        if self.title.is_none()
                            && self.screen == Screen::Playing
                            && self.player.is_some()
                        {
                            self.perf
                                .frame(elapsed.as_secs_f32() * 1000.0, &frame_spans);
                            if startup::filling() {
                                let mut fill: Vec<(&'static str, usize)> =
                                    self.streaming.stages().to_vec();
                                fill.push(("resident", self.world.chunks.len()));
                                fill.push(("uploads", self.pending_uploads.len()));
                                fill.push(("backlog", self.remote_backlog()));
                                startup::counters("world.fill", &fill);
                            }
                            if startup::on() {
                                startup::view_filled(
                                    self.world.chunks.len(),
                                    self.streaming.pending()
                                        + self.pending_uploads.len()
                                        + self.remote_backlog(),
                                    self.streaming.radius.max(0) as u32,
                                );
                            }
                            self.world_frames += 1;
                        }
                        self.frames += 1;
                        self.fps_frames += 1;
                        horizon_client::crash::debug_crash_tick();
                        if self.fps_at.elapsed() >= Duration::from_secs(1) {
                            self.fps = self.fps_frames as f32 / self.fps_at.elapsed().as_secs_f32();
                            self.fps_frames = 0;
                            self.fps_at = Instant::now();
                        }
                        self.frame_end = Instant::now();
                        if self.options.frames.is_some_and(|f| self.frames >= f)
                            || self
                                .options
                                .world_frames
                                .is_some_and(|f| self.world_frames >= f)
                        {
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }

            Ok(())
        })();
        if let Some(post) = mouse_post
            && result.is_ok()
        {
            self.screen_mouse_post(post);
        }
        // ScreenEvent.CharacterTyped.Pre: GLFW's char callback comes after its
        // key callback, so a key's characters reach the screen the key left
        // open, after the key's own effects (`KeyboardHandler.charTyped`).
        self.screen_chars_typed(typed.as_deref());
        if let Err(e) = result {
            self.error = Some(e);
            self.exit_on_error(event_loop);
        }
    }

    /// The loop ended: close the pads' output, save the world.
    pub(super) fn shut_down(&mut self) {
        if self.error.is_none()
            && let Some(renderer) = &mut self.renderer
        {
            renderer.drop_hdr_marker();
        }
        self.end_screenshot_sequence(Err("cancelled: the game is closing".into()));
        self.pad_close_output();
        if let Err(e) = self.save_for_exit() {
            log::warn!("Horizon: failed to save the world on exit: {e}");
        }
        // The client's last moment: its world is left and saved, its options and
        // resources are still held. A dedicated server and a splitscreen guest skip
        // it, as `client_setup` does.
        if self.options.is_client_start() {
            self.native_client_stopping();
        }
    }

    /// winit's `about_to_wait`: what happens between frames (stop requests, the closing
    /// save, world entry, the control socket, pads), then the next frame.
    pub(super) fn idle(&mut self, event_loop: &dyn EventLoopCtl) {
        // A splitscreen player's game has nothing to save: P1's shuts the process down.
        if horizon_threads::stop_requests() > 0
            && !self.options.split_guest
            && self.pending_save.as_ref().is_none_or(|save| !save.quit)
        {
            // SIGTERM, SIGINT or SIGHUP: the shutdown hook's server halt, as a window close.
            if let Err(error) = self.begin_closing_save(true) {
                self.error = Some(error);
                event_loop.exit();
                return;
            }
        }
        if self.pending_save.is_some() {
            match self.poll_closing_save() {
                Ok(true) => event_loop.exit(),
                Ok(false) => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    // Yield between polls even when disk I/O is slow.
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(4),
                    ));
                }
                Err(error) => {
                    self.error = Some(error);
                    self.exit_on_error(event_loop);
                }
            }
            return;
        }
        if let Err(error) = horizon_client::spans::time("world_entry", || self.resume_world_entry())
        {
            self.error = Some(error);
            self.exit_on_error(event_loop);
            return;
        }
        // The world a backup made before its upgrade opens (EditWorldScreen's callback).
        if let Some(folder) = self.title.as_mut().and_then(|t| t.take_open_request())
            && let Err(error) = self.open_world(&folder, None, None, false, None)
        {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        // A Realm's address came back (GetServerDetailsTask hands over to ConnectScreen).
        if let Some(action) = self.title.as_mut().and_then(|t| t.take_connect_request())
            && let Err(error) = self.title_action(event_loop, action)
        {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        horizon_client::spans::time("server_packs", || self.tick_server_packs());
        horizon_client::spans::time("control", || self.poll_control(event_loop));
        if self.pending_save.is_some() {
            return;
        }
        horizon_client::spans::time("pads", || self.poll_pads(event_loop));
        self.settle_after_run(event_loop);
        self.pause_if_inactive();
        if let Some(limit) = self.options.time_limit()
            && self.started.elapsed() > limit
        {
            self.error = Some(format!("run timed out after {} s", limit.as_secs()).into());
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            if self.options.offscreen {
                // Offscreen frames don't wait for the window to be drawn: an
                // occluded window (a locked macOS session) gets no redraws.
                let id = w.id();
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::RedrawRequested,
                    input_trace::InputOrigin::Game,
                );
            } else {
                w.request_redraw();
            }
        } else if self.options.headless {
            // The headless loop paces the frames itself (`run_headless`).
            self.handle_window_event(
                event_loop,
                WindowId::dummy(),
                WindowEvent::RedrawRequested,
                input_trace::InputOrigin::Game,
            );
            return;
        }
        // Consume the last frame's result: a hidden compositor may stop sending
        // redraws altogether, leaving a successful result stale indefinitely.
        event_loop.set_control_flow(event_loop::redraw_flow(
            &mut self.frame_drawn,
            self.options.offscreen,
            Instant::now(),
        ));
    }
}

impl ApplicationHandler for Game<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        debug_keys::warm_debug_cpu();
        // The Window constructor: created fullscreen when options.txt or --fullscreen says so.
        let fullscreen = self.settings.fullscreen || self.options.fullscreen;
        let size = self.options.window_size.map(f64::from);
        self.fullscreen = horizon_client::window::FullscreenState::new(fullscreen);
        let attributes =
            horizon_client::window::with_icon(Window::default_attributes().with_title("Horizon"));
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "horizon", "horizon",
        );
        // A game an agent started opens without the focus, as splitscreen's windows do: the
        // person keeps their keyboard and pointer until they click it (`unclaimed_window`).
        // X11 activates it anyway, and says so with a `Focused`.
        let agent_started = self.options.control.is_some();
        self.unclaimed_window = agent_started && !self.options.offscreen;
        // `--offscreen` renders into a texture: its window would only show white.
        let window_step = startup::step("window.create");
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.offscreen)
                .with_active(!agent_started),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        horizon_client::window::settle_created_fullscreen(&window, fullscreen);
        drop(window_step);
        match self.new_renderer(window.clone()) {
            Ok(r) => {
                self.renderer = Some(r);
                self.world_renderer_ready = true;
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        self.resize_state = Some(self.resize_state_of(window.inner_size()));
        self.window = Some(window);
        self.ime_allowed = false;
        self.ime_area = None;
        self.apply_settings();
        self.previous = Instant::now();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let key = if self.window.as_ref().is_some_and(|w| w.id() == id)
            && !self.scripted_run()
            && let WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } = &event
        {
            Some((event.clone(), glfw_modifiers(self.modifiers)))
        } else {
            None
        };
        self.handle_window_event(event_loop, id, event, input_trace::InputOrigin::User);
        if let Some((event, modifiers)) = key {
            let name = match event.physical_key {
                PhysicalKey::Code(code) => horizon_client::keybinds::key_name(code),
                PhysicalKey::Unidentified(_) => None,
            };
            let action = match (event.state, event.repeat) {
                (winit::event::ElementState::Released, _) => mod_api::KeyAction::Release,
                (_, false) => mod_api::KeyAction::Press,
                (_, true) => mod_api::KeyAction::Repeat,
            };
            // Still the key's handling: a mod's key hook may reach the clipboard.
            self.with_user_input(true, |game| {
                game.native_input_key(name.unwrap_or("key.keyboard.unknown"), action, modifiers);
            });
        }
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        // A composited view renders offscreen but its keyboard/mouse owner still
        // controls a visible native window. A headless offscreen run never does.
        if self.pending_save.is_none()
            && !self.scripted_run()
            && self.captured
            && self.focused
            && self.keyboard_mouse
            && (!self.options.offscreen || self.split_window)
            && self.screen == Screen::Playing
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            if self.pad_radial_captures() {
                return;
            }
            if x != 0.0 || y != 0.0 {
                self.pad_yield_input(event_loop);
            }
            // MouseHandler.onMove with the pointer grabbed; applied once per frame.
            self.note_input(
                input_trace::InputOrigin::User,
                input_trace::TraceKind::Motion { x, y },
            );
            self.mouse.on_move(0.0, 0.0, x, y, true, true);
            self.last_input = Instant::now();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.idle(event_loop);
    }
    /// The application is ending. On macOS, Quit (Cmd+Q) ends it through
    /// applicationWillTerminate without a CloseRequested; vanilla's GLFW
    /// window turns it into a window close, so Minecraft.stop saves the world.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.shut_down();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use input_trace::InputOrigin;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::{Ime, Modifiers, TouchPhase};
    use winit::keyboard::ModifiersState;

    // A key's `KeyEvent` can't be made outside winit; these are the person's other inputs.
    fn inputs() -> Vec<WindowEvent> {
        let device_id = DeviceId::dummy();
        vec![
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                button: MouseButton::Left,
            },
            WindowEvent::MouseWheel {
                device_id,
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: TouchPhase::Moved,
            },
            WindowEvent::CursorMoved {
                device_id,
                position: PhysicalPosition::new(10.0, 20.0),
            },
            WindowEvent::ModifiersChanged(Modifiers::from(ModifiersState::empty())),
            WindowEvent::Ime(Ime::Commit("a".into())),
        ]
    }

    #[test]
    fn a_person_is_locked_out_of_a_scripted_run_only() {
        for event in inputs() {
            assert!(locks_out(InputOrigin::User, true, &event), "{event:?}");
            assert!(!locks_out(InputOrigin::User, false, &event), "{event:?}");
        }
    }

    #[test]
    fn what_the_run_sends_still_plays() {
        for event in inputs() {
            assert!(!locks_out(InputOrigin::Agent, true, &event), "{event:?}");
            assert!(!locks_out(InputOrigin::Game, true, &event), "{event:?}");
        }
    }

    #[test]
    fn a_person_can_still_close_the_window() {
        for event in [
            WindowEvent::CloseRequested,
            WindowEvent::Resized(PhysicalSize::new(854, 480)),
        ] {
            assert!(!locks_out(InputOrigin::User, true, &event), "{event:?}");
            assert_eq!(
                held_focus(InputOrigin::User, true, &event),
                None,
                "{event:?}"
            );
        }
    }

    #[test]
    fn a_persons_focus_change_waits_for_the_run_to_end() {
        for focused in [true, false] {
            let event = WindowEvent::Focused(focused);
            assert_eq!(held_focus(InputOrigin::User, true, &event), Some(focused));
            assert_eq!(held_focus(InputOrigin::User, false, &event), None);
            assert!(!locks_out(InputOrigin::User, true, &event));
        }
    }
}
