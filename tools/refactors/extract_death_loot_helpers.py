"""Pure helper extraction to make room for the death-loot context boundary."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
creatures = root / 'crates/horizon-client/src/game/mobs/creatures/mod.rs'
shared = root / 'crates/horizon-client/src/game/native_hooks/shared.rs'


def extract(path, begin, end, destination, declaration, visibility=None):
    text = path.read_text()
    start = text.index(begin)
    stop = text.index(end, start)
    moved = text[start:stop]
    if visibility:
        moved = moved.replace(*visibility, 1)
    target = path.parent / destination
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text('use super::*;\n\n' + moved)
    path.write_text(text[:start] + declaration + '\n\n' + text[stop:])


extract(creatures, '/// Damage types of a projectile:', '/// A dying entity whose items drop',
        'death_source.rs', 'mod death_source;\npub(in crate::game) use death_source::death_hurt_source;')
extract(shared, '/// `view` as the screen', '/// What `decide_screen` says',
        'shared/screen_view.rs', '#[path = "shared/screen_view.rs"]\nmod screen_view;\nuse screen_view::screen_view_with_id;',
        ('fn screen_view_with_id', 'pub(super) fn screen_view_with_id'))
