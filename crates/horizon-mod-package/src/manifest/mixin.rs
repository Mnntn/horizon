//! `[[native.mixin]]`: a source-level edit of a game function. Each row is one horizon-mixins
//! `[[injection]]`, forwarded to the pinned tool, which owns the vocabulary beyond the keys
//! named here. Only `[native]` tables compile game code, so a wasm mod cannot declare one.
use serde::{Deserialize, Serialize};
use std::fmt;

/// One `[[native.mixin]]` row. `options` holds every other key of the tool's `[[injection]]`
/// (`selector`, `ordinal`, `bindings`, `cancellable`, `require`, `slice`, `variable`...), which
/// the tool validates when the game is built.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Mixin {
    pub id: String,
    /// The game function's path, as the pinned source spells it.
    pub target: String,
    pub kind: Kind,
    /// The helper's source file in this package, compiled inside the target function.
    pub helper: String,
    #[serde(flatten)]
    pub options: toml::Table,
}

/// A horizon-mixins table forwarded whole (`[[native.bridge]]`, `[[native.state]]`); only its
/// id is the package's to check.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Forwarded {
    pub id: String,
    #[serde(flatten)]
    pub fields: toml::Table,
}

/// horizon-mixins' injection kinds, spelled in upper case in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Head,
    Return,
    BeforeCall,
    AfterCall,
    AfterAssign,
    ModifyArg,
    ModifyArgs,
    ModifyConstant,
    ModifyVariable,
    ModifyExpressionValue,
    WrapOperation,
    Overwrite,
}

impl Kind {
    pub const ALL: [Self; 12] = [
        Self::Head,
        Self::Return,
        Self::BeforeCall,
        Self::AfterCall,
        Self::AfterAssign,
        Self::ModifyArg,
        Self::ModifyArgs,
        Self::ModifyConstant,
        Self::ModifyVariable,
        Self::ModifyExpressionValue,
        Self::WrapOperation,
        Self::Overwrite,
    ];

    /// The manifest spelling, which is the tool's.
    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "HEAD",
            Self::Return => "RETURN",
            Self::BeforeCall => "BEFORE_CALL",
            Self::AfterCall => "AFTER_CALL",
            Self::AfterAssign => "AFTER_ASSIGN",
            Self::ModifyArg => "MODIFY_ARG",
            Self::ModifyArgs => "MODIFY_ARGS",
            Self::ModifyConstant => "MODIFY_CONSTANT",
            Self::ModifyVariable => "MODIFY_VARIABLE",
            Self::ModifyExpressionValue => "MODIFY_EXPRESSION_VALUE",
            Self::WrapOperation => "WRAP_OPERATION",
            Self::Overwrite => "OVERWRITE",
        }
    }
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = std::borrow::Cow::<str>::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "unknown mixin kind `{name}`, expected one of {}",
                    Self::ALL.map(Self::name).join(", ")
                ))
            })
    }
}

/// Why a package's mixins are refused before any tool runs; `id` is `package:mixin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixinError {
    Invalid {
        id: String,
        field: &'static str,
        fix: &'static str,
    },
}

impl fmt::Display for MixinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { id, field, fix } => {
                write!(f, "E_MIXIN_INVALID: {id}: invalid {field}; {fix}")
            }
        }
    }
}
impl std::error::Error for MixinError {}

impl Mixin {
    pub(super) fn check_all(
        entries: &[Self],
        manifest: &super::Manifest,
    ) -> Result<(), MixinError> {
        for entry in entries {
            let invalid = |field, fix| MixinError::Invalid {
                id: format!("{}:{}", manifest.id, entry.id),
                field,
                fix,
            };
            if entry.target.trim().is_empty() {
                return Err(invalid("target", "name the game function's path"));
            }
            // A mixin edits the game's source. Another mod's functions are reached at its mod points.
            if manifest.mod_point_dependency(&entry.target).is_some() {
                return Err(invalid(
                    "target",
                    "a mixin targets a game function; inject at another mod's point with [[inject]]",
                ));
            }
            if !super::plain_path(&entry.helper)
                || !entry.helper.starts_with("src/")
                || !entry.helper.ends_with(".rs")
            {
                return Err(invalid(
                    "helper",
                    "name a Rust file under src/ in this package",
                ));
            }
            if entry.options.contains_key("hook") || entry.options.contains_key("hook_version") {
                return Err(invalid(
                    "hook",
                    "a named hook is a mod_api::Hooks method; a mixin names its target",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Manifest, inject};
    use super::*;

    const HEADER: &str =
        "id='sample'\nname='Sample'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n";
    const ROW: &str = "[[native.mixin]]\nid='autosave'\ntarget='game::Game::next_autosave'\nkind='MODIFY_EXPRESSION_VALUE'\nhelper='src/longer.rs'\n";

    fn native() -> String {
        format!(
            "[native]\ngame='26.3'\nhorizon='{0}'\nmixins='{0}'\n",
            "a".repeat(40)
        )
    }

    /// A `[[native.*]]` table follows `[native]`; a top-level one precedes it.
    fn parse(code: &str, rows: &str) -> Result<Manifest, String> {
        Manifest::parse(&if rows.starts_with("[[native.") {
            format!("{HEADER}{code}{rows}")
        } else {
            format!("{HEADER}{rows}{code}")
        })
    }

    #[test]
    fn each_table_accepts_only_its_own_kinds() {
        let inject_row = |kind: &str| {
            let call = if inject::Kind::ALL
                .iter()
                .any(|k| k.name() == kind && k.slot() == inject::SlotKind::Site)
            {
                "call='self.step'\nnth=0\nctx='c'\n"
            } else {
                ""
            };
            format!("[[inject]]\nid='a'\nat='game::tick'\nkind='{kind}'\nwith='run'\n{call}")
        };
        let mixin_row = |kind: &str| ROW.replace("MODIFY_EXPRESSION_VALUE", kind);
        let injects = inject::Kind::ALL.map(inject::Kind::name);
        let mixins = Kind::ALL.map(Kind::name);
        assert_eq!(
            injects,
            [
                "head",
                "return",
                "replace",
                "before_call",
                "after_call",
                "modify_value",
                "wrap_call",
                "wrap_if"
            ]
        );
        assert_eq!(
            mixins,
            [
                "HEAD",
                "RETURN",
                "BEFORE_CALL",
                "AFTER_CALL",
                "AFTER_ASSIGN",
                "MODIFY_ARG",
                "MODIFY_ARGS",
                "MODIFY_CONSTANT",
                "MODIFY_VARIABLE",
                "MODIFY_EXPRESSION_VALUE",
                "WRAP_OPERATION",
                "OVERWRITE"
            ]
        );
        let removed = ["modify_local", "expression", "field"];
        for kind in injects.iter().chain(&mixins).chain(&removed) {
            assert_eq!(
                parse(&native(), &inject_row(kind)).is_ok(),
                injects.contains(kind),
                "[[inject]] {kind}"
            );
            assert_eq!(
                parse(&native(), &mixin_row(kind)).is_ok(),
                mixins.contains(kind),
                "[[native.mixin]] {kind}"
            );
        }
        let error = parse(&native(), &mixin_row("head")).unwrap_err();
        assert!(
            error.contains("unknown mixin kind `head`, expected one of HEAD, RETURN"),
            "{error}"
        );
    }

    #[test]
    fn only_a_native_mod_declares_a_mixin() {
        let wasm = format!(
            "[wasm]\nsha512='{}'\ntoolchain='nightly'\n",
            "a".repeat(128)
        );
        // With or without the pins `[native]` needs, a wasm manifest is told why.
        for code in [format!("{wasm}{}", native()), wasm] {
            let error = parse(&code, ROW).unwrap_err();
            assert!(
                error.ends_with(
                    "a mod is [native] or [wasm], not both; only [native] tables compile game \
                     code, so a wasm mod has no mixin, bridge, state or module"
                ),
                "{error}"
            );
        }
        // The table has one spelling: a top-level row is an unknown table.
        let error = parse(&native(), &ROW.replace("[[native.mixin]]", "[[mixin]]")).unwrap_err();
        assert!(error.contains("unknown field `mixin`"), "{error}");
        assert!(parse(&native(), ROW).is_ok());
    }

    #[test]
    fn the_tools_keys_pass_through_and_the_packages_are_checked() {
        let row = format!(
            "{ROW}selector='self.interval'\nordinal=1\nrequire=1\ncancellable=true\npriority=5\nbindings=[{{name='n',ty='u32',value='n'}}]\nslice={{from={{selector='a',ordinal=0,require=1}}}}\nvariable={{name='n',ordinal=0,at='STORE'}}\n"
        );
        let manifest = parse(&native(), &row).unwrap();
        let mixin = &manifest.native.as_ref().unwrap().mixin[0];
        assert_eq!(mixin.kind, Kind::ModifyExpressionValue);
        // What the tool reads back is the row as written.
        let forwarded: toml::Table = toml::from_str(&toml::to_string(mixin).unwrap()).unwrap();
        let written: toml::Table = toml::from_str(&row).unwrap();
        assert_eq!(
            toml::Value::Table(forwarded),
            written["native"]["mixin"].as_array().unwrap()[0]
        );
        for (old, new, field) in [
            ("target='game::Game::next_autosave'", "target=' '", "target"),
            ("helper='src/longer.rs'", "helper='../longer.rs'", "helper"),
            ("helper='src/longer.rs'", "helper='/etc/passwd'", "helper"),
            (
                "helper='src/longer.rs'",
                "helper='src/longer.toml'",
                "helper",
            ),
            ("helper='src/longer.rs'", "helper='longer.rs'", "helper"),
            ("kind=", "hook='tick'\nkind=", "hook"),
            ("kind=", "hook_version=1\nkind=", "hook"),
        ] {
            let error = parse(&native(), &ROW.replace(old, new)).unwrap_err();
            assert!(
                error.contains(&format!(
                    "E_MIXIN_INVALID: sample:autosave: invalid {field};"
                )),
                "{error}"
            );
        }
        for missing in [
            "id='autosave'\n",
            "target='game::Game::next_autosave'\n",
            "helper='src/longer.rs'\n",
        ] {
            assert!(
                parse(&native(), &ROW.replace(missing, "")).is_err(),
                "{missing}"
            );
        }
    }

    #[test]
    fn a_mixin_never_targets_another_mods_function() {
        let row = ROW.replace("game::Game::next_autosave", "chem_lib::transfer");
        let dependency = "[dependencies]\nchem_lib='^1.0'\n";
        // Without the dependency the path is a game path for the tool to resolve.
        assert!(parse(&native(), &row).is_ok());
        let error = parse(&format!("{dependency}{}", native()), &row).unwrap_err();
        assert!(
            error.ends_with("E_MIXIN_INVALID: sample:autosave: invalid target; a mixin targets a game function; inject at another mod's point with [[inject]]"),
            "{error}"
        );
        // The inject form at that mod's point stays accepted.
        let inject = "[[inject]]\nid='double'\nat='chem_lib::transfer'\nkind='head'\nwith='double'\nhash='h'\n";
        assert!(parse(&format!("{dependency}{}", native()), inject).is_ok());
    }

    #[test]
    fn hostile_shapes_are_refused_without_panicking() {
        let wasm = format!(
            "[wasm]\nsha512='{}'\ntoolchain='nightly'\n",
            "a".repeat(128)
        );
        let inject = "[[inject]]\nid='a'\nat='game::tick'\nkind='head'\nwith='run'\n";
        let shapes = [
            // a kind that is no string, or an empty one, in either table
            inject.replace("kind='head'", "kind=3"),
            inject.replace("kind='head'", "kind=''"),
            inject.replace("kind='head'", "kind=['head']"),
            ROW.replace("kind='MODIFY_EXPRESSION_VALUE'", "kind=3"),
            ROW.replace("kind='MODIFY_EXPRESSION_VALUE'", "kind=''"),
            // tables where arrays of tables belong, and the reverse
            "[inject]\nid='a'\nat='game::tick'\nkind='head'\nwith='run'\n".into(),
            "inject='head'\n".into(),
            "inject=[1, 'two', []]\n".into(),
            ROW.replace("[[native.mixin]]", "[native.mixin]"),
            "[[native]]\ngame='26.3'\n".into(),
            "[[native.bridge]]\nid=7\n".into(),
            "[[native.state]]\nid=['a']\n".into(),
            // keys of the wrong type, blank ids, and ids that are not ASCII
            ROW.replace("helper='src/longer.rs'", "helper=1"),
            ROW.replace("helper='src/longer.rs'", "helper={path='src/longer.rs'}"),
            ROW.replace("target='game::Game::next_autosave'", "target=[]"),
            ROW.replace("id='autosave'", "id=' '"),
            ROW.replace("id='autosave'", "id=''"),
            inject.replace("id='a'", "id=' '"),
            "[[native.bridge]]\nid=' '\n".into(),
            "[[native.state]]\nid='\t'\n".into(),
            ROW.replace("helper='src/longer.rs'", "helper='src/\u{0}.rs\\..'"),
            // a duplicate key
            format!("{ROW}kind='HEAD'\n"),
            format!("{inject}kind='return'\n"),
        ];
        for shape in &shapes {
            for code in [native(), wasm.clone(), String::new()] {
                assert!(parse(&code, shape).is_err(), "{shape}{code}");
            }
        }
        // Unicode in an id is the tool's to judge, and is carried as written.
        let manifest = parse(&native(), &ROW.replace("id='autosave'", "id='épée-✓'")).unwrap();
        assert_eq!(manifest.native.unwrap().mixin[0].id, "épée-✓");
    }

    #[test]
    fn the_native_or_wasm_refusal_does_not_mask_another_error() {
        let wasm = format!(
            "[wasm]\nsha512='{}'\ntoolchain='nightly'\n",
            "a".repeat(128)
        );
        // The wasm table itself is wrong: that stays the error, whatever `[native]` holds.
        let broken = wasm.replace("toolchain='nightly'\n", "toolchain=3\n");
        for code in [broken.clone(), format!("{broken}{}", native())] {
            let error = parse(&code, ROW).unwrap_err();
            assert!(!error.contains("not both"), "{error}");
        }
        let error = parse(&format!("{broken}{}", native()), ROW).unwrap_err();
        assert!(error.contains("invalid type"), "{error}");
        // Not TOML at all.
        let error = parse(&format!("{wasm}= ="), ROW).unwrap_err();
        assert!(!error.contains("not both"), "{error}");
    }

    #[test]
    fn mixins_bridges_states_modules_and_injects_share_one_id_space() {
        let tables = [
            ROW.to_owned(),
            "[[inject]]\nid='autosave'\nat='game::tick'\nkind='head'\nwith='run'\n".into(),
            "[[native.module]]\nid='autosave'\nfile='game.rs'\nname='runtime'\nsource='src/runtime.rs'\n".into(),
            "[[native.bridge]]\nid='autosave'\nkind='GET'\ntarget='game::Game'\nmember='world'\n".into(),
            "[[native.state]]\nid='autosave'\ntarget='game::Game'\nty='u32'\ninitial='0'\nconstructor=[]\n".into(),
        ];
        for (a, first) in tables.iter().enumerate() {
            // Top-level rows precede `[native]`; its own tables follow it.
            let text = |second: &str| {
                let (top, nested): (Vec<&str>, Vec<&str>) = [first.as_str(), second]
                    .into_iter()
                    .partition(|table| !table.starts_with("[[native."));
                format!("{HEADER}{}{}{}", top.concat(), native(), nested.concat())
            };
            assert!(Manifest::parse(&text("")).is_ok(), "{first}");
            for second in &tables[a + 1..] {
                let error = Manifest::parse(&text(second)).unwrap_err();
                assert!(error.contains("duplicate"), "{first}{second}: {error}");
                assert!(
                    Manifest::parse(&text(&second.replace("id='autosave'", "id='other'"))).is_ok(),
                    "{first}{second}"
                );
            }
        }
        let manifest = Manifest::parse(&format!(
            "{HEADER}{}{}{}",
            native(),
            tables[3],
            tables[4].replace("id='autosave'", "id='count'")
        ))
        .unwrap();
        let native = manifest.native.unwrap();
        assert_eq!(native.bridge[0].fields["member"].as_str(), Some("world"));
        assert_eq!(native.state[0].fields["ty"].as_str(), Some("u32"));
    }
}
