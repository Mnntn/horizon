//! Supported, versioned tool view of the resolved mod interface.
use crate::{Catalog, FillMode};
use quote::ToTokens;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

/// Serialize the same API event types and point shapes used by code generation.
pub fn sdk_catalog(root: &Path, api_version: &str) -> Result<String, String> {
    let catalog = Catalog::load(root)?;
    let mut code = crate::sources(root, FillMode::Ignore)?.hooks;
    let render = crate::dispatch::render(root, &catalog)?;
    code.push_str(&render.render_dispatch);
    let file = syn::parse_file(&code).map_err(|e| e.to_string())?;
    let records: BTreeMap<_, _> = file
        .items
        .into_iter()
        .filter_map(|item| {
            let syn::Item::Struct(item) = item else {
                return None;
            };
            Some((item.ident.to_string(), item.fields))
        })
        .collect();
    let mut hooks = Vec::new();
    for hook in catalog.hooks {
        let event = format!("{}Event", crate::hook_id(&hook.name));
        let fields = records
            .get(&event)
            .ok_or_else(|| format!("missing generated event {event}"))?;
        let output_name = hook
            .out
            .as_deref()
            .or_else(|| (hook.contract == crate::Contract::Cancel).then_some("cancelled"));
        let mut arguments = Vec::new();
        let mut output = Value::Null;
        for field in fields {
            let name = field
                .ident
                .as_ref()
                .ok_or("unnamed event field")?
                .to_string();
            let ty = field.ty.to_token_stream().to_string();
            if Some(name.as_str()) == output_name {
                output = json!({"name": name, "type": ty});
            } else {
                arguments.push(json!({"name": name, "type": ty}));
            }
        }
        hooks.push(json!({"id": hook.name, "side": match hook.side { crate::catalog::Side::Server => "server", crate::catalog::Side::Client => "client", crate::catalog::Side::Both => "both" },
            "scope": hook.scope.map(|scope| json!({"kind": match scope.kind { crate::catalog::PartKind::Block => "block", crate::catalog::PartKind::BlockEntity => "blockentity", crate::catalog::PartKind::Item => "item", crate::catalog::PartKind::Menu => "menu", crate::catalog::PartKind::Fluid => "fluid" }, "key": scope.key})),
            "contract": match hook.contract { crate::Contract::Listen => "listen", crate::Contract::Cancel => "cancel", crate::Contract::Collect => "collect", crate::Contract::Replace => "replace" }, "arguments": arguments, "output": output}));
    }
    let points = crate::points::game_points(root)?
        .into_iter()
        .map(|point| {
            let access = match (point.api_refusal.is_none(), point.raw) {
                (true, true) => vec!["api", "internals"],
                (true, false) => vec!["api"],
                (false, true) => vec!["internals"],
                (false, false) => vec![],
            };
            let mut value = serde_json::to_value(point).map_err(|e| e.to_string())?;
            value["access"] = json!(access);
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    serialize(api_version, hooks, points)
}

fn serialize(
    api_version: &str,
    mut hooks: Vec<Value>,
    mut points: Vec<Value>,
) -> Result<String, String> {
    hooks.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    points.sort_by(|a, b| {
        (a["name"].as_str(), a["target"].as_str()).cmp(&(b["name"].as_str(), b["target"].as_str()))
    });
    serde_json::to_string_pretty(
        &json!({"schema_version": 1, "api_version": api_version, "hooks": hooks, "points": points}),
    )
    .map(|s| s + "\n")
    .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "sdk_catalog_tests.rs"]
mod tests;
