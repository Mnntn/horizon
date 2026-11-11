//! One constructor emitter for native and wasm API injection listeners.
use horizon_mod_package::{
    Manifest,
    manifest::inject::{Access, Inject, Kind},
};

/// The part of a staged `points.json` entry needed to resolve an API listener constructor.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Point {
    /// The constructor's head-place name.
    pub name: String,
    /// The function's logical path.
    pub target: String,
    /// Whether an API return place exists (not a borrowed return).
    pub at_return: bool,
    /// Why its API listeners are refused: the first parameter with no API type, named.
    #[serde(default)]
    pub api_refusal: Option<String>,
    /// What its listeners may do with the return: `cancel`, `result` (an API value) or `none`.
    #[serde(default)]
    pub returns: String,
    /// Its API listeners' host: `none`, or the host they take.
    #[serde(default)]
    pub host: String,
    /// Its raw form, read for the return type a refused `replace` names.
    #[serde(default)]
    pub raw_shape: Option<RawReturn>,
    /// The function's return type as written: a `cancel` point takes a `replace` only when it
    /// is `()`.
    #[serde(default)]
    pub return_ty: Option<String>,
    /// The table module its constructors are in: none for the game's `mod_points`, or a mod
    /// point's (#2339) table as the generated code names it (`mod_0::__horizon_points`).
    #[serde(default)]
    pub table: Option<String>,
}

/// The part of a point's raw contract a `replace` needs: an API one names its return type when
/// refused, an internals one is built from its outcome and receiver.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct RawReturn {
    /// The function's return type, as written.
    pub return_ty: String,
    /// What the raw record holds for the outcome: `cancel`, `result` or `none` (a borrowed
    /// return, which no head listener can make).
    #[serde(default)]
    pub outcome: String,
    /// The raw listener's receiver.
    #[serde(default)]
    pub receiver: RawReceiver,
}

/// The receiver a raw listener gets.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct RawReceiver {
    /// `none` for a free or associated function, else `shared` or `mut`.
    pub kind: String,
}

/// An internals `replace` at `point`'s head, through `mod_points::replace!`'s internals arm.
/// `constructor` is the mod facade's raw head constructor and `listener` the `with` function's
/// path. Refused, naming the point and type, where the raw record can't carry the function's
/// outcome: a borrowed return (no `result`), and a `cancel` whose function returns anything but
/// `()` (a `Result<(), E>` cancel answers `Ok(())`, so a replace couldn't produce `Err`).
pub fn internals_replace(
    entry: &Inject,
    mod_id: &str,
    point: &Point,
    constructor: &str,
    listener: &str,
) -> Result<String, String> {
    let (id, at) = (&entry.id, &entry.at);
    let raw = point
        .raw_shape
        .as_ref()
        .ok_or_else(|| format!("{id}: the point catalog lists no raw form for {at}; rebuild it"))?;
    let outcome = match raw.outcome.as_str() {
        "result" => "result",
        "cancel" if plain_path(&raw.return_ty) == "()" => "cancel",
        _ => {
            return Err(format!(
                "{id}: no internals replace at {at}: a replace returns the function's value, \
                 and the raw record can't carry `{}`",
                raw.return_ty
            ));
        }
    };
    let receiver = if raw.receiver.kind == "none" {
        "none"
    } else {
        "receiver"
    };
    Ok(format!(
        "mod_points::replace!(internals {constructor}, {outcome}, {receiver}, {mod_id:?}, {}, {id:?}, {listener})",
        entry.priority
    ))
}

/// Remove type arguments and whitespace exactly as the native launcher resolves point paths.
pub fn plain_path(path: &str) -> String {
    let mut depth = 0usize;
    path.chars()
        .filter(|&c| match c {
            '<' => {
                depth += 1;
                false
            }
            '>' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0 && !c.is_whitespace(),
        })
        .collect()
}

/// Emit API head/return constructor expressions in manifest order, without surrounding syntax.
/// `prefix` is the compiler-owned Rust module path preceding each manifest `with` function,
/// for example `mod_0::` in the native launcher or `original_mod::` in a wasm wrapper.
/// A `replace` goes through `mod_points::replace!`, which owns its contract for both targets.
/// Call-site injections and internals listeners are emitted by their own slot routes.
pub fn api(manifest: &Manifest, points: &[Point], prefix: &str) -> Result<Vec<String>, String> {
    Ok(resolve(manifest, points)?
        .into_iter()
        .map(|(place, entry, point)| {
            let (id, priority, inject) = (&manifest.id, entry.priority, &entry.id);
            let listener = format!("{prefix}{}", entry.with);
            let table = point.table.as_deref().unwrap_or("mod_points");
            let constructor = format!("{table}::listeners::{place}");
            if entry.kind == Kind::Replace {
                let host = if point.host == "none" { "none" } else { "host" };
                format!(
                    "mod_points::replace!({constructor}, {}, {host}, {id:?}, {priority}, {inject:?}, {listener})",
                    point.returns
                )
            } else {
                format!("{constructor}({id:?}, {priority}, {inject:?}, {listener})")
            }
        })
        .collect())
}

/// An internals listener at a mod point (#2339): `table`'s typed constructor for `place`, which
/// checks `listener` against the place's raw listener type, in the order its injection asks.
pub fn mod_internals(
    table: &str,
    place: &str,
    mod_id: &str,
    listener: &str,
    priority: i32,
    inject: &str,
) -> String {
    format!("{table}::raw::{place}({mod_id:?}, {listener}).ordered({priority}, {inject:?})")
}

/// Place names selected by the same resolver as constructor emission. The wasm build passes
/// these to the points crate so it compiles only the guest record codecs and exports it uses.
pub fn places(manifest: &Manifest, points: &[Point]) -> Result<Vec<String>, String> {
    Ok(resolve(manifest, points)?
        .into_iter()
        .map(|(place, _, _)| place)
        .collect())
}

type Resolved<'a, 'p> = (
    String,
    &'a horizon_mod_package::manifest::inject::Inject,
    &'p Point,
);

fn resolve<'a, 'p>(
    manifest: &'a Manifest,
    points: &'p [Point],
) -> Result<Vec<Resolved<'a, 'p>>, String> {
    manifest
        .inject
        .iter()
        .filter(|entry| entry.access == Access::Api && entry.call.is_none())
        .map(|entry| {
            if !matches!(entry.kind, Kind::Head | Kind::Replace | Kind::Return) {
                return Err(format!(
                    "{}: expected an API head, return or replace injection",
                    entry.id
                ));
            }
            let point = points
                .iter()
                .find(|point| plain_path(&point.target) == plain_path(&entry.at))
                .ok_or_else(|| format!("{}: unknown API point {}", entry.id, entry.at))?;
            if let Some(why) = &point.api_refusal {
                return Err(format!(
                    "{}: no API listener at {}: {why}",
                    entry.id, entry.at
                ));
            }
            if entry.kind == Kind::Return && !point.at_return {
                return Err(format!("{}: no public API place at {}", entry.id, entry.at));
            }
            if entry.kind == Kind::Replace {
                replaceable(&entry.id, &entry.at, point)?;
            }
            let place = if entry.kind == Kind::Return {
                format!("{}_return", point.name)
            } else {
                point.name.clone()
            };
            Ok((place, entry, point))
        })
        .collect()
}

/// A `replace` listener produces the function's outcome, so its point must return `()` (the
/// cancel is forced) or a value the API carries. A `cancel` point returning anything else
/// (`Result<(), E>`, whose cancel would answer `Ok(())`) is refused, and so is a catalog that
/// doesn't say. The refusal names the point and its return type.
pub fn replaceable(id: &str, at: &str, point: &Point) -> Result<(), String> {
    if point.host.is_empty() {
        return Err(format!(
            "{id}: the point catalog lists no listener host for {at}; rebuild it"
        ));
    }
    let ty = match point.returns.as_str() {
        "result" => return Ok(()),
        "cancel" => match point.return_ty.as_deref() {
            Some("()") => return Ok(()),
            Some(ty) => format!("`{ty}`"),
            None => {
                return Err(format!(
                    "{id}: the point catalog lists no return type for {at}; rebuild it"
                ));
            }
        },
        _ => point
            .raw_shape
            .as_ref()
            .map_or("its return type".to_owned(), |raw| {
                format!("`{}`", raw.return_ty)
            }),
    };
    Err(format!(
        "{id}: no replace listener at {at}: a replace returns the function's value, and {ty} \
         has no API form"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_and_wasm_share_the_existing_constructor_expressions() {
        let manifest = Manifest::parse(
            r#"
id = "fixture"
name = "Fixture"
version = "0.1.0"
api = "0.2"
capabilities = ["registries"]
[native]
game = "26.3"
horizon = "0000000000000000000000000000000000000000"
mixins = "0000000000000000000000000000000000000000"
[[inject]]
id = "before"
at = "horizon_client::game::Game<'_>::tick"
kind = "head"
with = "head"
access = "api"
priority = 12
hash = "pin"
[[inject]]
id = "after"
at = "horizon_client::game::Game::tick"
kind = "return"
with = "tail"
access = "api"
hash = "pin"
"#,
        )
        .unwrap();
        let points = [Point {
            name: "tick".into(),
            target: "horizon_client::game::Game < '_ > :: tick".into(),
            at_return: true,
            api_refusal: None,
            returns: "cancel".into(),
            host: "both".into(),
            raw_shape: None,
            return_ty: Some("()".into()),
            table: None,
        }];
        let native = api(&manifest, &points, "mod_3::")
            .unwrap()
            .iter()
            .map(|expression| format!("        {expression},\n"))
            .collect::<String>();
        assert_eq!(
            native,
            concat!(
                "        mod_points::listeners::tick(\"fixture\", 12, \"before\", mod_3::head),\n",
                "        mod_points::listeners::tick_return(\"fixture\", 0, \"after\", mod_3::tail),\n"
            )
        );
        // A replace owns the outcome, by the point's return and host, through one macro.
        let mut replace = manifest.clone();
        replace.inject[0].kind = Kind::Replace;
        assert_eq!(
            api(&replace, &points, "mod_3::").unwrap()[0],
            "mod_points::replace!(mod_points::listeners::tick, cancel, host, \"fixture\", 12, \"before\", mod_3::head)"
        );
        let mut free = points[0].clone();
        free.returns = "result".into();
        free.host = "none".into();
        assert_eq!(
            api(&replace, &[free], "original_mod::").unwrap()[0],
            "mod_points::replace!(mod_points::listeners::tick, result, none, \"fixture\", 12, \"before\", original_mod::head)"
        );
        let mut opaque = points[0].clone();
        opaque.returns = "none".into();
        opaque.raw_shape = Some(RawReturn {
            return_ty: "Frame<'_>".into(),
            ..RawReturn::default()
        });
        assert_eq!(
            api(&replace, &[opaque], "mod_3::").unwrap_err(),
            "before: no replace listener at horizon_client::game::Game<'_>::tick: a replace returns the function's value, and `Frame<'_>` has no API form"
        );
        let mut unlisted = points[0].clone();
        unlisted.host = String::new();
        assert!(api(&replace, &[unlisted], "mod_3::").is_err());
        // A `Result<(), E>` point cancels with `Ok(())`: a replace there couldn't produce `Err`.
        let mut fallible = points[0].clone();
        fallible.return_ty = Some("Result<(),String>".into());
        assert_eq!(
            api(&replace, &[fallible], "mod_3::").unwrap_err(),
            "before: no replace listener at horizon_client::game::Game<'_>::tick: a replace returns the function's value, and `Result<(),String>` has no API form"
        );
        let mut untyped = points[0].clone();
        untyped.return_ty = None;
        assert!(api(&replace, &[untyped], "mod_3::").is_err());
        // A mod point's constructor is in its own table.
        let mut own = points[0].clone();
        own.table = Some("mod_1::__horizon_points".into());
        assert_eq!(
            api(&manifest, &[own.clone()], "mod_3::").unwrap()[0],
            "mod_1::__horizon_points::listeners::tick(\"fixture\", 12, \"before\", mod_3::head)"
        );
        assert_eq!(
            api(&replace, &[own], "mod_3::").unwrap()[0],
            "mod_points::replace!(mod_1::__horizon_points::listeners::tick, cancel, host, \"fixture\", 12, \"before\", mod_3::head)"
        );
        assert_eq!(
            mod_internals(
                "mod_1::__horizon_points",
                "tick_return",
                "fixture",
                "mod_3::seen",
                -2,
                "seen"
            ),
            "mod_1::__horizon_points::raw::tick_return(\"fixture\", mod_3::seen).ordered(-2, \"seen\")"
        );
        let wasm = api(&manifest, &points, "original_mod::").unwrap();
        assert_eq!(
            wasm[0],
            "mod_points::listeners::tick(\"fixture\", 12, \"before\", original_mod::head)"
        );
        let mut head_only = points[0].clone();
        head_only.at_return = false;
        assert!(api(&manifest, &[head_only], "mod_3::").is_err());
        assert!(api(&manifest, &[], "mod_3::").is_err());
        let mut refused = points[0].clone();
        refused.api_refusal =
            Some("argument 1 `batch` (`&mutHudBatch`) has no API type yet".into());
        assert_eq!(
            api(&manifest, &[refused], "mod_3::").unwrap_err(),
            "before: no API listener at horizon_client::game::Game<'_>::tick: argument 1 `batch` (`&mutHudBatch`) has no API type yet"
        );
    }

    #[test]
    fn an_internals_replace_follows_the_raw_outcome_and_receiver() {
        let manifest = Manifest::parse(
            r#"
id = "fixture"
name = "Fixture"
version = "0.1.0"
api = "0.2"
capabilities = ["registries"]
[native]
game = "26.3"
horizon = "0000000000000000000000000000000000000000"
mixins = "0000000000000000000000000000000000000000"
[[inject]]
id = "own"
at = "horizon_client::game::Game<'_>::tick"
kind = "replace"
with = "own"
access = "internals"
priority = -3
hash = "pin"
"#,
        )
        .unwrap();
        let entry = &manifest.inject[0];
        let point = |outcome: &str, return_ty: &str, receiver: &str| Point {
            name: "tick".into(),
            table: None,
            target: "horizon_client::game::Game<'_>::tick".into(),
            at_return: true,
            api_refusal: None,
            returns: "none".into(),
            host: "both".into(),
            raw_shape: Some(RawReturn {
                return_ty: return_ty.into(),
                outcome: outcome.into(),
                receiver: RawReceiver {
                    kind: receiver.into(),
                },
            }),
            return_ty: None,
        };
        let emit = |p: &Point| {
            internals_replace(
                entry,
                "fixture",
                p,
                "mod_0_internals::raw::tick",
                "mod_0::own",
            )
        };
        assert_eq!(
            emit(&point("cancel", "()", "mut")).unwrap(),
            "mod_points::replace!(internals mod_0_internals::raw::tick, cancel, receiver, \"fixture\", -3, \"own\", mod_0::own)"
        );
        assert_eq!(
            emit(&point("result", "Frame < '_ >", "none")).unwrap(),
            "mod_points::replace!(internals mod_0_internals::raw::tick, result, none, \"fixture\", -3, \"own\", mod_0::own)"
        );
        assert!(
            emit(&point("cancel", "()", "shared"))
                .unwrap()
                .contains(", receiver,")
        );
        assert_eq!(
            emit(&point("cancel", "Result < () >", "mut")).unwrap_err(),
            "own: no internals replace at horizon_client::game::Game<'_>::tick: a replace returns the function's value, and the raw record can't carry `Result < () >`"
        );
        assert!(
            emit(&point("none", "&mut usize", "mut"))
                .unwrap_err()
                .contains("`&mut usize`")
        );
        let mut unlisted = point("result", "u32", "none");
        unlisted.raw_shape = None;
        assert!(emit(&unlisted).unwrap_err().contains("no raw form"));
    }
}
