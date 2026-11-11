//! Read-only classification of a native mod set, without executing mod or game code.
use super::*;

/// Plan enabled native mods for an instance. Worker-only filesystem and release reads.
pub fn routes(home: &Path, base: Option<&str>) -> Result<Routes, Failed> {
    require_consent()?;
    let lease = crate::native_cache::Cache::acquire(&backend::build_dir()).map_err(cache_error)?;
    let Some(folder) = stage_installed(home, root().join("installed.routes"))? else {
        return Ok(Routes::default());
    };
    classify(&folder, base, &lease)
}

/// Share classification with an actual build under its existing cache lease.
pub(super) fn classify(
    folder: &Path,
    base: Option<&str>,
    _lease: &crate::native_cache::Cache,
) -> Result<Routes, Failed> {
    let found = scan_bundles(folder)?;
    let mut result = Routes::default();
    let mut projects = Vec::new();
    for bundle in &found {
        let route = match api::Project::read(bundle) {
            Ok(project) => {
                let route = prebuilt::project_refusal(&project)
                    .map_or(Route::Prebuilt, |reason| Route::Game { reason });
                projects.push(project);
                route
            }
            Err(error) => Route::Invalid {
                reason: error.to_string(),
            },
        };
        result.mods.push((bundle.manifest.id.clone(), route));
    }
    if projects.is_empty() {
        return Ok(result);
    }
    let base = base.ok_or_else(|| tr("Install Horizon first.").to_string())?;
    let revision = build_revision(base, checked_about(base)?.as_ref())?;
    if let Some(list) = mismatch(&found, &revision) {
        return Err(mismatch_error(&list).into());
    }
    result.unavailable = prebuilt::prebuilt_availability(&revision, Target::Client);
    Ok(result)
}
