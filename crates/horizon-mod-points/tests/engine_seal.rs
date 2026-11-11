//! An empty host still closes platform registration through the engine entry point.
#[test]
fn engine_seals_even_when_no_listener_was_registered() {
    horizon_engine_slots::install_seal(mod_points::__private::seal)
        .expect("provider installed before engine startup");
    horizon_engine_slots::seal();
    assert!(!horizon_engine_slots::POINTS.active(mod_points::PointId::Tick as usize));
    let listener = mod_points::listeners::tick("late", 0, "head", |_, _| {});
    assert_eq!(
        mod_points::register(&listener),
        Err(mod_points::RegisterError::Sealed)
    );
}
