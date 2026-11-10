//! Transactional freshness updates; never changes the game pin.
use super::*;
use horizon_mod_api_gen::sites::identity::{CallIdentity, Selection, select};

pub(super) fn run(dir: &Path, args: &[String]) -> Result<(), String> {
    let mut accept = Vec::new();
    for pair in args.chunks(2) {
        match pair {
            [flag, id] if flag == "--accept" => accept.push(id.as_str()),
            _ => return Err("usage: cargo horizon-mod inject update [--accept <id>]…".into()),
        }
    }
    let path = dir.join(MANIFEST);
    let original = crate::bounded::read(&path, crate::bounded::MAX_MANIFEST)?;
    let manifest = Manifest::parse(&original)?;
    let native = manifest
        .native
        .as_ref()
        .ok_or("inject update needs a [native] mod")?;
    let mut internals = crate::internals::load(dir, &native.horizon)?;
    let cache = dir.join("target/horizon-mod/prebuilt");
    let points = cache.join(&native.horizon).join("points.json");
    let catalog: Catalog =
        serde_json::from_str(&crate::bounded::read(&points, crate::bounded::MAX_CATALOG)?)
            .map_err(|e| e.to_string())?;
    // A declaration at a dependency's function is checked against that mod's points (#2339).
    let mut libraries = std::collections::BTreeSet::new();
    for inject in &manifest.inject {
        let krate = inject.at.split("::").next().unwrap_or_default();
        if !libraries.insert(krate) {
            continue;
        }
        if let Some(library) = super::library(dir, &inject.at, &catalog.wire)? {
            internals.extend(library.points.into_iter().map(|point| ListedInternal {
                path: point.target,
                kind: "fn".into(),
                cfg: String::new(),
                signature: point.signature,
                hash: point.hash,
                shape_only: false,
            }));
        }
    }
    // Older cached snapshots provide the signature behind a declaration's old hash.
    let mut old = Vec::new();
    for entry in fs::read_dir(cache).map_err(|e| e.to_string())? {
        let path = entry
            .map_err(|e| e.to_string())?
            .path()
            .join("internals.json");
        let text = match crate::bounded::read(&path, crate::bounded::MAX_CATALOG) {
            Ok(text) => Some(text),
            Err(e) if e.is_not_found() => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(text) = text
            && let Ok(snapshot) = serde_json::from_str::<Vec<ListedInternal>>(&text)
        {
            old.extend(snapshot);
        }
    }
    let (candidate, reports, refused) =
        plan(&original, &manifest, &internals, &catalog, &old, &accept)?;
    for report in reports {
        println!("{report}");
    }
    if refused {
        return Err("inject update refused; no files changed".into());
    }
    if candidate != original {
        crate::internals::atomic_write_manifest(&path, candidate)?;
    }
    Ok(())
}

/// The catalog entry an inject targets: a function or method, or a struct for a field.
fn target<'a>(
    internals: &'a [ListedInternal],
    inject: &horizon_mod_package::manifest::inject::Inject,
) -> Option<&'a ListedInternal> {
    let kind = if inject.kind == Kind::Field {
        "struct"
    } else {
        "fn"
    };
    internals.iter().find(|i| {
        i.path == inject.at
            && !i.shape_only
            && (i.kind == kind || (kind == "fn" && i.kind == "method"))
    })
}

/// What moving the pin from the `old` catalog to the `new` one does to the injects that carry
/// no hash, which follow the pin: a changed signature is refused as `E_INJECT_CHANGED` unless
/// its id is in `accept`, exactly as a stale pinned hash is refused at build, and a target the
/// new game lacks is `E_INJECT_MISSING`. Returns the reports and whether to refuse.
pub(crate) fn pin_moves(
    manifest: &Manifest,
    old: &[ListedInternal],
    new: &[ListedInternal],
    accept: &[&str],
) -> (Vec<String>, bool) {
    let mut reports = Vec::new();
    let mut refused = false;
    for inject in manifest
        .inject
        .iter()
        .filter(|inject| inject.hash.is_none())
    {
        let id = format!("{}:{}", manifest.id, inject.id);
        let Some(now) = target(new, inject) else {
            reports.push(
                InjectError::Missing {
                    id,
                    at: inject.at.clone(),
                }
                .to_string(),
            );
            refused = true;
            continue;
        };
        let before = target(old, inject);
        if before.is_some_and(|before| before.hash == now.hash) {
            continue;
        }
        reports.push(
            InjectError::Changed {
                id,
                at: inject.at.clone(),
                old: before.map_or_else(
                    || "absent from the old game".to_owned(),
                    |before| format!("{} ({})", before.hash, before.signature),
                ),
                new: format!("{} ({})", now.hash, now.signature),
            }
            .to_string(),
        );
        refused |= !accept.contains(&inject.id.as_str());
    }
    (reports, refused)
}

fn plan(
    text: &str,
    manifest: &Manifest,
    internals: &[ListedInternal],
    catalog: &Catalog,
    old: &[ListedInternal],
    accept: &[&str],
) -> Result<(String, Vec<String>, bool), String> {
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{MANIFEST}: {e}"))?;
    let mut reports = Vec::new();
    let mut refused = false;
    let mut edited = false;
    for id in accept {
        if !manifest.inject.iter().any(|i| i.id == *id) {
            reports.push(format!("unknown inject id {id}"));
            refused = true;
        }
    }
    for (index, inject) in manifest.inject.iter().enumerate() {
        let id = format!("{}:{}", manifest.id, inject.id);
        let Some(item) = target(internals, inject) else {
            reports.push(
                InjectError::Missing {
                    id,
                    at: inject.at.clone(),
                }
                .to_string(),
            );
            refused = true;
            continue;
        };
        let item = match horizon_mod_package::internals::resolve(
            internals,
            &id,
            &inject.at,
            Some(&item.kind),
        ) {
            Ok(item) => item,
            Err(error) => {
                reports.push(error.to_string());
                refused = true;
                continue;
            }
        };
        let mut nth = None;
        if inject.ctx.is_some() {
            let identities: Vec<_> = catalog
                .sites
                .iter()
                .filter(|s| s.target == inject.at)
                .map(|s| CallIdentity {
                    call: &s.call,
                    nth: s.ordinal,
                    ctx: &s.ctx,
                })
                .collect();
            match select(
                &identities,
                inject.call.as_deref().unwrap_or(""),
                inject.nth.unwrap_or(0),
                inject.ctx.as_deref().unwrap_or(""),
            ) {
                Selection::Selected(_) => (),
                Selection::Moved(n) => {
                    reports.push(
                        InjectError::Moved {
                            id: id.clone(),
                            nth: n,
                        }
                        .to_string(),
                    );
                    nth = Some(n);
                }
                Selection::Lost | Selection::Ambiguous => {
                    reports.push(
                        InjectError::Lost {
                            id,
                            at: inject.at.clone(),
                        }
                        .to_string(),
                    );
                    refused = true;
                    continue;
                }
            }
        }
        // A declaration without a hash follows the pin: there is nothing of its own to refresh.
        let pinned = inject.hash.as_deref();
        let changed = pinned.is_some_and(|hash| hash != item.hash);
        if let Some(hash) = pinned.filter(|_| changed) {
            let previous = old
                .iter()
                .find(|i| i.path == inject.at && i.hash == hash)
                .map(|i| i.signature.as_str())
                .unwrap_or("signature unavailable in cached snapshots");
            reports.push(
                InjectError::Changed {
                    id,
                    at: inject.at.clone(),
                    old: format!("{hash} ({previous})"),
                    new: format!("{} ({})", item.hash, item.signature),
                }
                .to_string(),
            );
            if !accept.contains(&inject.id.as_str()) {
                refused = true;
                continue;
            }
        }
        let table = doc
            .get_mut("inject")
            .and_then(|i| i.as_array_of_tables_mut())
            .and_then(|a| a.get_mut(index))
            .ok_or("missing inject table")?;
        // Replacing the scalar payload retains its existing comments and spacing.
        for (key, replacement) in [
            (
                "hash",
                changed.then(|| toml_edit::Value::from(item.hash.as_str())),
            ),
            (
                "nth",
                nth.map(|n| i64::try_from(n).map(toml_edit::Value::from))
                    .transpose()
                    .map_err(|_| "call ordinal exceeds TOML integer range")?,
            ),
        ] {
            if let Some(mut replacement) = replacement {
                edited = true;
                if let Some(existing) = table.get(key).and_then(|i| i.as_value()) {
                    *replacement.decor_mut() = existing.decor().clone();
                }
                table[key] = toml_edit::Item::Value(replacement);
            }
        }
    }
    let candidate = if refused || !edited {
        text.to_owned()
    } else if text.contains("\r\n") {
        doc.to_string().replace('\n', "\r\n")
    } else {
        doc.to_string()
    };
    Manifest::parse(&candidate)?;
    Ok((candidate, reports, refused))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{TEXT, edit, fixture, reviewer_directory};
    use super::*;
    fn pinned() -> String {
        edit(
            TEXT,
            "add game::tick --kind wrap_call --call game::first --with wrap",
        )
        .unwrap()
    }
    fn check(
        text: &str,
        items: &[ListedInternal],
        catalog: &Catalog,
        accept: &[&str],
    ) -> (String, Vec<String>, bool) {
        plan(
            text,
            &Manifest::parse(text).unwrap(),
            items,
            catalog,
            &[],
            accept,
        )
        .unwrap()
    }
    fn disk_fixture(label: &str, text: &str, hash: &str, sites: &str) -> std::path::PathBuf {
        let dir = reviewer_directory(label);
        fs::write(dir.join(MANIFEST), text).unwrap();
        disk_catalog(&dir, hash, sites);
        dir
    }
    fn disk_catalog(dir: &Path, hash: &str, sites: &str) {
        let cache =
            dir.join("target/horizon-mod/prebuilt/0123456789012345678901234567890123456789");
        fs::write(cache.join("internals.json"), format!(r#"[{{"path":"game::tick","kind":"method","signature":"unsafe fn tick(&self, value: bool) -> bool","hash":"{hash}"}}]"#)).unwrap();
        fs::write(cache.join("points.json"), format!(r#"{{"sites":{sites}}}"#)).unwrap();
    }
    const MOVED: &str =
        r#"[{"target":"game::tick","call":"game::first","ordinal":7,"ctx":"context-zero"}]"#;
    #[test]
    fn inject_update_disk_movement_preserves_comments() {
        let text = pinned().replace("nth = 0", "nth = 0 # ordinal");
        let dir = disk_fixture("update-moved", &text, "signature-fixture", MOVED);
        run(&dir, &[]).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(MANIFEST)).unwrap(),
            text.replace("nth = 0", "nth = 7")
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn inject_update_disk_accept_and_unsafe_rewrite_refusal() {
        let text = pinned();
        let dir = disk_fixture("update-accept", &text, "new-signature", MOVED);
        assert!(run(&dir, &[]).is_err());
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        run(&dir, &["--accept".into(), "wrap".into()]).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join(MANIFEST)).unwrap(),
            text.replace("signature-fixture", "new-signature")
                .replace("nth = 0", "nth = 7")
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn inject_update_disk_accept_never_retargets_or_creates() {
        let text = pinned();
        let dir = disk_fixture("update-lost", &text, "new-signature", "[]");
        for sites in [
            "[]",
            r#"[{"target":"game::tick","call":"game::first","ordinal":0,"ctx":"other-context"}]"#,
            r#"[{"target":"game::tick","call":"game::first","ordinal":0,"ctx":"context-zero"},{"target":"game::tick","call":"game::first","ordinal":2,"ctx":"context-zero"}]"#,
        ] {
            disk_catalog(&dir, "new-signature", sites);
            assert!(run(&dir, &["--accept".into(), "wrap".into()]).is_err());
            assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        }
        disk_catalog(&dir, "new-signature", MOVED);
        assert!(run(&dir, &["--accept".into(), "unknown".into()]).is_err());
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        let cache =
            dir.join("target/horizon-mod/prebuilt/0123456789012345678901234567890123456789");
        fs::write(cache.join("internals.json"), "[]").unwrap();
        assert!(run(&dir, &["--accept".into(), "wrap".into()]).is_err());
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn inject_update_disk_refused_second_entry_blocks_first() {
        let text = edit(&pinned(), "add game::tick --kind head --with second").unwrap();
        let dir = disk_fixture("update-partial", &text, "new-signature", MOVED);
        assert!(run(&dir, &["--accept".into(), "wrap".into()]).is_err());
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn inject_update_unavailable_old_signature_is_explicit() {
        let text = pinned();
        let (mut items, catalog) = fixture();
        items[0].hash = "new-signature".into();
        let (_, reports, refused) = check(&text, &items, &catalog, &[]);
        assert!(refused);
        assert!(reports[0].contains("signature unavailable in cached snapshots"));
        let dir = disk_fixture("update-unavailable", &text, "new-signature", MOVED);
        assert!(run(&dir, &[]).is_err());
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn inject_update_changed_report_shows_old_and_new_signatures() {
        let text = pinned();
        let (mut items, catalog) = fixture();
        let mut old = items.clone();
        old[0].signature = "fn tick(&self, value: bool)".into();
        items[0].hash = "new-signature".into();
        let (_, reports, refused) = plan(
            &text,
            &Manifest::parse(&text).unwrap(),
            &items,
            &catalog,
            &old,
            &[],
        )
        .unwrap();
        assert!(refused);
        let expected = "game::tick signature-fixture (fn tick(&self, value: bool)) -> \
                        new-signature (fn tick(&mut self)); run inject update";
        assert!(reports[0].starts_with("E_INJECT_CHANGED: "));
        assert!(reports[0].ends_with(expected), "{}", reports[0]);
    }
    #[test]
    fn moving_the_pin_refuses_a_changed_function_an_unpinned_inject_follows() {
        let text =
            format!("{TEXT}[[inject]]\nid='tick'\nat='game::tick'\nkind='head'\nwith='tick'\n");
        let manifest = Manifest::parse(&text).unwrap();
        assert_eq!(manifest.inject[0].hash, None);
        let (old, _) = fixture();
        // The same signature on the new game: nothing to say, nothing refused.
        assert_eq!(pin_moves(&manifest, &old, &old, &[]), (vec![], false));
        let mut new = old.clone();
        new[0].hash = "new-signature".into();
        new[0].signature = "fn tick(&mut self, value: u32)".into();
        let (reports, refused) = pin_moves(&manifest, &old, &new, &[]);
        assert!(refused);
        assert_eq!(
            reports,
            [
                "E_INJECT_CHANGED: example:tick: game::tick signature-fixture (fn tick(&mut self)) -> \
              new-signature (fn tick(&mut self, value: u32)); run inject update"
            ]
        );
        // Accepting it takes the change; the declaration stays without a hash.
        assert!(!pin_moves(&manifest, &old, &new, &["tick"]).1);
        let (reports, refused) = pin_moves(&manifest, &old, &[], &["tick"]);
        assert!(refused && reports[0].starts_with("E_INJECT_MISSING: example:tick"));
        // A pinned inject is left to the build's own check, as before.
        let pinned = Manifest::parse(&format!("{text}hash='signature-fixture'\n")).unwrap();
        assert_eq!(pin_moves(&pinned, &old, &new, &[]), (vec![], false));
    }

    #[test]
    fn inject_update_noop_leaves_inode_and_mtime() {
        let dir = reviewer_directory("update-noop");
        let path = dir.join(MANIFEST);
        fs::write(&path, edit(TEXT, "add game::tick --kind head").unwrap()).unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        fs::hard_link(&path, dir.join("old")).unwrap();
        run(&dir, &[]).unwrap();
        assert_eq!(before, fs::metadata(&path).unwrap().modified().unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(&path).unwrap().ino(),
                fs::metadata(dir.join("old")).unwrap().ino()
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn reviewer_symlink_preserved() {
        use std::os::unix::fs::symlink;
        let text = pinned();
        let dir = disk_fixture("review-symlink", &text, "signature-fixture", MOVED);
        let path = dir.join(MANIFEST);
        fs::rename(&path, dir.join("real.toml")).unwrap();
        symlink("real.toml", &path).unwrap();
        run(&dir, &[]).unwrap();
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(dir.join("real.toml")).unwrap(),
            text.replace("nth = 0", "nth = 7")
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn reviewer_unrelated_corrupt_snapshot() {
        let text = pinned();
        let dir = disk_fixture("review-corrupt", &text, "signature-fixture", MOVED);
        let old = dir.join("target/horizon-mod/prebuilt/old");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("internals.json"), "broken").unwrap();
        run(&dir, &[]).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn reviewer_crlf_noop() {
        let text = pinned().replace("\n", "\r\n");
        let (items, catalog) = fixture();
        assert_eq!(check(&text, &items, &catalog, &[]).0, text);
        let dir = disk_fixture(
            "review-crlf",
            &text,
            "signature-fixture",
            r#"[{"target":"game::tick","call":"game::first","ordinal":0,"ctx":"context-zero"}]"#,
        );
        let before = fs::metadata(dir.join(MANIFEST))
            .unwrap()
            .modified()
            .unwrap();
        run(&dir, &[]).unwrap();
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        assert_eq!(
            before,
            fs::metadata(dir.join(MANIFEST))
                .unwrap()
                .modified()
                .unwrap()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reviewer_loader_error_refuses() {
        let text = TEXT.to_owned();
        let dir = disk_fixture("review-loader", &text, "signature-fixture", MOVED);
        fs::write(dir.join("target/horizon-mod/prebuilt/0123456789012345678901234567890123456789/internals.json"), "bad").unwrap();
        assert!(run(&dir, &[]).is_err());
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn reviewer_accept_boundary_and_refusal_metadata() {
        use std::os::unix::fs::MetadataExt;
        let text = pinned();
        let dir = disk_fixture("review-boundary", &text, "signature-fixture", MOVED);
        let path = dir.join(MANIFEST);
        let before = fs::metadata(&path).unwrap();
        for args in [
            vec!["--accept".into(), "".into()],
            vec!["--accept".into(), "x".repeat(100000)],
            vec!["--accept".into()],
        ] {
            assert!(run(&dir, &args).is_err());
            let after = fs::metadata(&path).unwrap();
            assert_eq!(before.ino(), after.ino());
            assert_eq!(before.modified().unwrap(), after.modified().unwrap());
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
        run(
            &dir,
            &[
                "--accept".into(),
                "wrap".into(),
                "--accept".into(),
                "wrap".into(),
            ],
        )
        .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
