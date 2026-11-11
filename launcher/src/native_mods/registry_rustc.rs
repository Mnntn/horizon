//! Controlled outer adapter: inject SDK externs before the existing compiler/cache chain.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, process::Command};

const PLAN: &str = "HORIZON_NATIVE_REGISTRY_RUSTC_PLAN";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub rustc: PathBuf,
    pub target: String,
    pub cache_wrapper: Option<PathBuf>,
    pub workspace_wrapper: Option<PathBuf>,
    pub search: Vec<PathBuf>,
    pub native: PathBuf,
    pub crates: BTreeMap<PathBuf, Bindings>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bindings {
    pub externs: BTreeMap<String, PathBuf>,
    pub link_args: Vec<String>,
}

impl Plan {
    pub fn configure(
        &self,
        command: &mut Command,
        path: &std::path::Path,
        adapter: &std::path::Path,
    ) -> Result<(), String> {
        std::fs::write(path, serde_json::to_vec(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        command.env(PLAN, path).env("RUSTC_WRAPPER", adapter);
        if let Some(wrapper) = &self.workspace_wrapper {
            command.env("RUSTC_WORKSPACE_WRAPPER", wrapper);
        }
        Ok(())
    }

    fn arguments(
        &self,
        mut args: Vec<OsString>,
        manifest: Option<PathBuf>,
    ) -> Result<(PathBuf, Vec<OsString>), String> {
        let compiler = args.first().ok_or("rustc adapter: missing compiler")?;
        let expected = self.workspace_wrapper.as_ref().unwrap_or(&self.rustc);
        // Cargo passes the prior workspace wrapper only for workspace invocations. Registry
        // and host dependencies call rustc directly through this same outer adapter.
        let workspace =
            std::path::Path::new(compiler) == expected && self.workspace_wrapper.is_some();
        let rustc_index = usize::from(workspace);
        if args.get(rustc_index).map(PathBuf::from).as_ref() != Some(&self.rustc) {
            return Err("rustc adapter: compiler is not the pinned toolchain".into());
        }
        let value = |key: &str| {
            args.windows(2)
                .find(|pair| pair[0] == key)
                .map(|pair| pair[1].clone())
        };
        let target = value("--target");
        let kind = value("--crate-type");
        if target.as_deref() == Some(std::ffi::OsStr::new(&self.target))
            && kind.as_deref() != Some(std::ffi::OsStr::new("proc-macro"))
        {
            if let Some(bindings) = manifest.as_ref().and_then(|path| self.crates.get(path)) {
                for path in &self.search {
                    args.push("-L".into());
                    args.push(format!("dependency={}", path.display()).into());
                }
                args.push("-L".into());
                args.push(format!("native={}", self.native.display()).into());
                args.push("-C".into());
                args.push("prefer-dynamic".into());
                for (name, path) in &bindings.externs {
                    args.push("--extern".into());
                    args.push(format!("{name}={}", path.display()).into());
                }
                for arg in &bindings.link_args {
                    args.push("-C".into());
                    args.push(format!("link-arg={arg}").into());
                }
            }
        }
        if let Some(cache) = &self.cache_wrapper {
            Ok((cache.clone(), args))
        } else {
            Ok((PathBuf::from(args.remove(0)), args))
        }
    }
}

pub(super) fn wrapper() -> Option<i32> {
    let path = std::env::var_os(PLAN)?;
    let result = (|| -> Result<i32, String> {
        let plan: Plan = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let (program, args) = plan.arguments(
            std::env::args_os().skip(1).collect(),
            std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from),
        )?;
        let status = Command::new(program)
            .args(args)
            .env_remove(PLAN)
            .status()
            .map_err(|e| e.to_string())?;
        Ok(status.code().unwrap_or(1))
    })();
    Some(match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan() -> Plan {
        Plan {
            rustc: "/pinned/rustc".into(),
            target: "game-target".into(),
            cache_wrapper: Some("/cache".into()),
            workspace_wrapper: Some("/workspace-wrapper".into()),
            search: vec!["/sdk/deps".into()],
            native: "/sdk/native".into(),
            crates: BTreeMap::from([(
                "/mod".into(),
                Bindings {
                    externs: BTreeMap::from([("mod_api".into(), "/sdk/api.rmeta".into())]),
                    link_args: vec!["-rpath".into()],
                },
            )]),
        }
    }
    #[test]
    fn only_target_workspace_code_receives_sdk_bindings_and_cache_chain_survives() {
        let plan = plan();
        for (manifest, target, kind, inject) in [
            ("/mod", "game-target", "lib", true),
            ("/mod", "game-target", "proc-macro", false),
            ("/registry", "game-target", "lib", false),
            ("/mod", "host-target", "lib", false),
        ] {
            let args: Vec<OsString> = [
                "/workspace-wrapper",
                "/pinned/rustc",
                "--target",
                target,
                "--crate-type",
                kind,
            ]
            .into_iter()
            .map(Into::into)
            .collect();
            let (program, output) = plan.arguments(args.clone(), Some(manifest.into())).unwrap();
            assert_eq!(program, PathBuf::from("/cache"));
            assert!(output.starts_with(&args));
            assert_eq!(
                output.iter().any(|arg| arg == "mod_api=/sdk/api.rmeta"),
                inject
            );
        }
        let args = vec![OsString::from("/pinned/rustc"), "-vV".into()];
        assert_eq!(plan.arguments(args.clone(), None).unwrap().1, args);
        assert!(plan
            .arguments(vec!["/other/rustc".into()], None)
            .unwrap_err()
            .contains("pinned"));
    }
    #[test]
    fn no_cache_invokes_workspace_wrapper_or_compiler_directly() {
        let mut plan = plan();
        plan.cache_wrapper = None;
        let (program, args) = plan
            .arguments(
                vec![
                    "/workspace-wrapper".into(),
                    "/pinned/rustc".into(),
                    "-vV".into(),
                ],
                None,
            )
            .unwrap();
        assert_eq!(program, PathBuf::from("/workspace-wrapper"));
        assert_eq!(args[0], "/pinned/rustc");
        assert_eq!(
            plan.arguments(vec!["/pinned/rustc".into(), "-vV".into()], None)
                .unwrap()
                .0,
            PathBuf::from("/pinned/rustc")
        );
    }
}
