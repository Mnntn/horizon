#!/usr/bin/env python3
"""Port GPU fixtures from raw point records to typed replacement installers."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
p = root / 'crates/horizon-client-render/fixtures/world_views.rs'
s = p.read_text()
a = s.index('fn ask_views(')
b = s.index('    {\n        let view = &mut *event.view;', a)
s = s[:a] + '''fn ask_views(views: &mut Vec<WorldView>) {
    views.extend(VIEWS.lock().unwrap().iter().copied());
}
fn view_drawn(view: &mut native_render::WorldViewDrawn<'_>) {
''' + s[b:].replace('        let view = &mut *event.view;\n', '', 1)
a = s.index('    use mod_points::')
b = s.index('\n}', a)
s = s[:a] + '''    assert!(native_render::__fnr_world_views(ask_views));
    assert!(native_render::__fnr_world_view_drawn(view_drawn));''' + s[b:]
p.write_text(s)
p = root / 'crates/horizon-client-render/fixtures/standard_pack_gpu_entities.rs'
s = p.read_text()
a = s.index('static CHECK_RAW_RECORD:')
b = s.index('\nfn pack(', a)
s = s[:a] + '''static OBSERVED: AtomicUsize = AtomicUsize::new(0);
fn observe_needs(needs: &mut native_render::RenderNeeds) {
    needs.entity_spans |= NEEDS_SPANS.load(Ordering::Relaxed);
}
fn observe_world(world: &mut native_render::WorldDrawn<'_>) {
    if NEEDS_SPANS.load(Ordering::Relaxed) {
        assert_eq!(world.entities.len(), 12);
        assert_eq!(world.spans.iter().map(|s| s.id).collect::<Vec<_>>(), [77, 88, 99]);
        OBSERVED.fetch_add(1, Ordering::Relaxed);
    }
}
fn install_observer() {
    assert!(native_render::__fnr_render_needs(observe_needs));
    assert!(native_render::__fnr_world_drawn(observe_world));
}
''' + s[b:]
a = s.index('fn slot_ns(')
b = s.index('fn fixture(', a)
s = s[:a] + s[b:]
a = s.index('    let empty_slots =')
b = s.index('    let white =', a)
s = s[:a] + '    install_observer();\n' + s[b:]
s = s.replace('            "raw_slots_empty": empty_slots, "raw_slots_subscribed": subscribed_slots,\n', '')
p.write_text(s)
p = root / 'tools/run_standard_pack_gpu_entities.py'
s = p.read_text()
s = '\n'.join(line for line in s.split('\n') if 'parser.add_argument("--raw-slots-only"' not in line)
s = s.replace(',\n                        *(["--raw-slots-only"] if args.raw_slots_only else [])', '')
p.write_text(s)
