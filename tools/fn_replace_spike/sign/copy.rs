#[allow(unused_imports)]
use horizon_client::game::sign_edit::*;
use horizon_client::game::{
    Game,
    commands::{Target, execute::Source},
};
use horizon_client::{
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    items::ItemStack,
    signs::{ClickAction, DYE_TEXT, click_action, editor_sprite},
    ui::EditBox,
};
use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
use winit::{
    event::{ElementState, KeyEvent},
    keyboard::{KeyCode, PhysicalKey},
};

pub fn draw_sign_editor(this: &Game<'_>, batch: &mut HudBatch) {
    let Some(edit) = &this.sign_edit else { return };
    let Some(BlockEntity::Sign(sign)) = this.world.block_entities.get(&edit.pos) else {
        return;
    };
    this.render_transparent_background("sign_editor", batch);
    let atlas = &this.gui.atlas;
    let (w, h) = (batch.width, batch.height);
    let (key, fallback) = if sign.hanging {
        ("hanging_sign.edit", "Edit Hanging Sign Message")
    } else {
        ("sign.edit", "Edit Sign Message")
    };
    let title = this.translate_with_fallback(key, fallback);
    // SignEditScreen / HangingSignEditScreen: their title.
    horizon_client::widgets::begin_screen(|| {
        horizon_client::widgets::ScreenInfo::new("sign_edit", title.clone())
    });
    let tw = HudBatch::text_width(atlas, &title);
    batch.text(atlas, &title, (w - tw) / 2.0, 40.0, u32::MAX, true);
    let facts = this.res.facts();
    let block = facts.get(this.world.block(edit.pos.0, edit.pos.1, edit.pos.2));
    let wood = facts.wood_type(&block.name).unwrap_or("oak");
    let cx = w / 2.0;
    let y = if sign.hanging {
        // HangingSignEditScreen: 16x16 texture scaled 4.5 at (0, -13).
        let s = 4.5;
        batch.sprite_scaled(
            atlas,
            &editor_sprite("hanging_signs", wood),
            cx - 8.0 * s,
            125.0 - 13.0 - 8.0 * s,
            16.0 * s,
            16.0 * s,
            u32::MAX,
        );
        125.0
    } else {
        // SignEditScreen: 24x26 texture scaled 3.9 at (0, 27); wall signs show 12 rows.
        let s = 3.9;
        let wall = block.sign_form().is_some_and(|f| f.wall);
        let rows = if wall { 12.0 } else { 26.0 };
        if let Some(mut r) = atlas.sprites.get(&editor_sprite("signs", wood)).copied() {
            r.v1 = r.v0 + (r.v1 - r.v0) * rows / 26.0;
            batch.rect_uv(
                cx - 12.0 * s,
                90.0 + 27.0 - 13.0 * s,
                24.0 * s,
                rows * s,
                r,
                u32::MAX,
            );
        }
        90.0
    };
    let text = if edit.front { &sign.front } else { &sign.back };
    let color = if text.glowing {
        text.text_color()
    } else {
        text.dark_color()
    };
    let color = rgba((color >> 16) as u8, (color >> 8) as u8, color as u8, 255);
    let line_height = sign.line_height();
    let midpoint = (4.0 * line_height / 2.0).floor();
    for (i, line) in text.lines.iter().enumerate() {
        let shown = if i == edit.line {
            edit.edit.value.as_str()
        } else {
            line.as_str()
        };
        let lw = HudBatch::text_width(atlas, shown);
        let x = cx - (lw as i32 / 2) as f32;
        let ly = y + i as f32 * line_height - midpoint;
        batch.text(atlas, shown, x, ly, color, false);
        if i != edit.line {
            continue;
        }
        let value = &edit.edit.value;
        if edit.edit.cursor != edit.edit.highlight {
            let (a, b) = (
                edit.edit.cursor.min(edit.edit.highlight),
                edit.edit.cursor.max(edit.edit.highlight),
            );
            let pre: String = value.chars().take(a).collect();
            let sel: String = value.chars().skip(a).take(b - a).collect();
            batch.text_highlight(
                atlas,
                (
                    x + HudBatch::text_width(atlas, &pre),
                    ly,
                    HudBatch::text_width(atlas, &sel),
                    line_height,
                ),
                true,
            );
        }
        let pre: String = value.chars().take(edit.edit.cursor).collect();
        let caret_x = x + HudBatch::text_width(atlas, &pre);
        if edit.edit.cursor_visible() {
            batch.fill(atlas, caret_x, ly, 1.0, line_height, color);
        }
        horizon_client::preedit::report_caret(caret_x, ly, line_height);
    }
    let (bx, by) = done_button(w, h);
    horizon_client::widgets::declare_action(
        horizon_client::widgets::Action::Done,
        [bx, by, 200.0, 20.0],
        true,
    );
    horizon_client::widgets::narratable(|| {
        horizon_client::widgets::Narratable::button(done_label(), [bx, by, 200.0, 20.0], true)
    });
    batch.sprite_scaled(
        atlas,
        "gui/sprites/widget/button",
        bx,
        by,
        200.0,
        20.0,
        u32::MAX,
    );
    let done = done_label();
    let tw = HudBatch::text_width(atlas, &done);
    batch.text(
        atlas,
        &done,
        bx + 100.0 - (tw / 2.0).floor(),
        by + 6.0,
        u32::MAX,
        true,
    );
}
