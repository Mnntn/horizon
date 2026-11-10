//! The wasm fixture's shutdown listener, registered natively: the same answers as `real_member`.
#[path = "fixture/src/matrix.rs"]
mod matrix;
#[path = "support/checks.rs"]
mod matrix_checks;
#[path = "support/no_hooks.rs"]
mod no_hooks;
#[path = "fixture/src/replace.rs"]
mod replace;
#[path = "fixture/src/shutdown.rs"]
mod shutdown;
#[path = "fixture/src/turn.rs"]
mod turn;

/// `horizon_client::game::server_lifecycle::remember_shutdown`, as the staging pass marks it.
#[horizon_mod_api_macros::point(name = "game_server_lifecycle_remember_shutdown", free)]
fn remember_shutdown(failure: &mut Option<String>, message: String) {
    if failure.is_none() {
        *failure = Some(message);
    }
}

/// `horizon_client::game::beacon::average`, as the staging pass marks it.
#[horizon_mod_api_macros::point(name = "game_beacon_average", free)]
fn average(a: u32, b: u32) -> u32 {
    replace::BODY_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    (a + b) / 2
}

/// `horizon_client::game::commands::wrap_degrees` (`Mth.wrapDegrees`), as the staging pass
/// marks it.
#[horizon_mod_api_macros::point(name = "game_commands_wrap_degrees", free)]
fn wrap_degrees(angle: f32) -> f32 {
    let wrapped = angle % 360.0;
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

#[test]
fn the_shared_listener_source_answers_natively_as_in_wasm() {
    matrix_checks::baseline();
    // Without a listener the body wraps the angle as given.
    assert_eq!(wrap_degrees(10.0), 10.0);
    // Without a listener the body averages.
    assert_eq!(average(3, 7), 5);
    assert_eq!(
        replace::BODY_CALLS.swap(0, std::sync::atomic::Ordering::SeqCst),
        1
    );
    // Without a listener the body runs as written.
    let mut failure = None;
    remember_shutdown(&mut failure, "disk full".into());
    assert_eq!(failure.as_deref(), Some("disk full"));
    mod_points::register(
        &mod_points::listeners::game_server_lifecycle_remember_shutdown(
            "native",
            0,
            "shutdown",
            shutdown::on_shutdown,
        )
        .ordered(0, "shutdown"),
    )
    .unwrap();
    mod_points::register(&mod_points::replace!(
        mod_points::listeners::game_beacon_average,
        result,
        none,
        "native",
        0,
        "average",
        replace::own_average
    ))
    .unwrap();
    mod_points::register(&mod_points::listeners::listener_fixture_sum_return(
        "native",
        0,
        "sum",
        matrix::sum_return,
    ))
    .unwrap();
    mod_points::register(&mod_points::listeners::listener_fixture_mirrors(
        "native",
        0,
        "mirrors",
        matrix::mirrors,
    ))
    .unwrap();
    // A head listener changing its argument (mixin `@ModifyVariable` on a parameter).
    mod_points::register(&mod_points::listeners::game_commands_wrap_degrees(
        "native",
        0,
        "turn",
        turn::turn_angle,
    ))
    .unwrap();
    mod_points::__private::seal();
    matrix_checks::check();
    for (before, message, after) in shutdown::CASES {
        let mut failure = before.map(String::from);
        remember_shutdown(&mut failure, message.into());
        assert_eq!(failure.as_deref(), after, "{message}");
    }
    for (a, b, value) in replace::CASES {
        assert_eq!(average(a, b), value, "{a} {b}");
        assert_eq!(
            replace::BODY_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }
    for (angle, wrapped) in turn::CASES {
        assert_eq!(wrap_degrees(angle), wrapped, "{angle}");
    }
}
