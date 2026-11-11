use super::*;

#[test]
fn slot_lists_are_a_sorted_union_with_locations_and_checked_syntax() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("slot-lists-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let a = directory.join("a.txt");
    let b = directory.join("b.txt");
    fs::write(
        &a,
        "# comment\ngame::Game<'_>::tick | self.step | 0 | abc # note\ngame::tick | .step | |\nlower::run\n",
    )
    .unwrap();
    fs::write(
        &b,
        "game::Game::tick | self.step | 0 | abc\nlower::run\nlower::second\nz::run | make | 1 | def\n",
    )
    .unwrap();
    let declarations = slots(&directory).unwrap();
    assert_eq!(declarations.calls.len(), 3);
    assert_eq!(declarations.calls[0].target, "game::Game::tick");
    assert_eq!(declarations.calls[0].ctx, "abc");
    assert!(declarations.calls[0].origin.ends_with("a.txt:2"));
    assert_eq!(declarations.calls[1].ordinal, None);
    assert!(declarations.calls[1].ctx.is_empty());
    assert_eq!(declarations.calls[2].ordinal, Some(1));
    assert_eq!(declarations.calls[2].ctx, "def");
    assert_eq!(
        declarations.functions.keys().collect::<Vec<_>>(),
        ["lower::run", "lower::second"]
    );
    for (bad, reason) in [
        ("game::tick | self.step", "expected at"),
        ("game::tick | self.step | 0", "expected at"),
        ("? | self.step | 0 |", "invalid syntax"),
        ("game::tick | . | 0 |", "selector"),
        ("game::tick | .step | -1 |", "ordinal"),
        ("type ?", "invalid syntax"),
        ("?", "invalid syntax"),
        ("type game::Thing | make | 0 |", "invalid syntax"),
    ] {
        fs::write(&a, bad).unwrap();
        let error = slots(&directory).unwrap_err();
        assert!(matches!(error.cause, Cause::Syntax(_) | Cause::Format(_)));
        assert!(error.origin.as_ref().unwrap().ends_with("a.txt:1"));
        assert!(error.to_string().contains(reason), "{error}");
    }
    fs::write(&a, "game::Game::tick | self.step | 0 | different\n").unwrap();
    let error = slots(&directory).unwrap_err().to_string();
    assert!(
        error.contains("a.txt:1") && error.contains("b.txt:1"),
        "{error}"
    );
    fs::remove_dir_all(&directory).unwrap();
    let empty = slots(&directory).unwrap();
    assert!(empty.calls.is_empty() && empty.functions.is_empty());
}
