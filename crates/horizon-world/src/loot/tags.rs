//! Engine-owned tag reads passed only while evaluating custom loot kinds.

/// Authoritative tag reads without a game or registry borrow.
/// Registry names are vanilla directory names (`item`, `block`, `fluid`).
pub trait LootTags {
    /// Members of one tag in its resolved order. Unknown tags are empty.
    fn members(&self, registry: &str, tag: &str) -> Vec<String>;
    /// Whether one registry value belongs to a tag.
    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.members(registry, tag)
            .iter()
            .any(|member| member == id)
    }
}

impl LootTags for super::LootData {
    fn members(&self, registry: &str, tag: &str) -> Vec<String> {
        self.tag(registry, tag)
    }
}

#[cfg(test)]
thread_local! { static STEP_BUDGET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }

/// Resolve a requested tag iteratively, preserving nested order without a depth limit.
/// Completed subgraphs are skipped: their leaves are already in the deduplicated output.
pub(super) fn members(
    tags: &std::collections::HashMap<(String, String), Vec<serde_json::Value>>,
    registry: &str,
    name: &str,
) -> Vec<String> {
    enum Step<'a> {
        Enter(&'a str),
        Exit(&'a str),
        Value(&'a serde_json::Value),
    }
    let mut pending = vec![Step::Enter(name)];
    let mut active = std::collections::HashSet::new();
    let mut completed = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    while let Some(step) = pending.pop() {
        #[cfg(test)]
        STEP_BUDGET.with(|budget| {
            if let Some(remaining) = budget.get() {
                budget.set(Some(
                    remaining
                        .checked_sub(1)
                        .expect("tag traversal repeated a completed subgraph"),
                ));
            }
        });
        match step {
            Step::Enter(name) => {
                if completed.contains(name) {
                    continue;
                }
                if !active.insert(name) {
                    return Vec::new();
                }
                pending.push(Step::Exit(name));
                if let Some(values) = tags.get(&(registry.to_owned(), name.to_owned())) {
                    pending.extend(values.iter().rev().map(Step::Value));
                }
            }
            Step::Exit(name) => {
                active.remove(name);
                completed.insert(name);
            }
            Step::Value(value) => {
                let Some(id) = value
                    .as_str()
                    .or_else(|| value.get("id").and_then(serde_json::Value::as_str))
                else {
                    continue;
                };
                if let Some(nested) = id.strip_prefix('#') {
                    pending.push(Step::Enter(nested));
                } else if seen.insert(id) {
                    result.push(id.to_owned());
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn repeated_shared_subtags_are_visited_once_without_exponential_work() {
        let mut tags = std::collections::HashMap::new();
        for index in 0..128 {
            let next = json!(format!("#test:{}", index + 1));
            tags.insert(
                ("item".into(), format!("test:{index}")),
                vec![next.clone(), next],
            );
        }
        tags.insert(
            ("item".into(), "test:128".into()),
            vec![json!("minecraft:stick")],
        );
        // A deterministic test-only work budget kills repeated traversal without a
        // timing assertion or allowing an exponential mutant to hang the suite.
        STEP_BUDGET.with(|budget| budget.set(Some(4096)));
        let result = members(&tags, "item", "test:0");
        STEP_BUDGET.with(|budget| budget.set(None));
        assert_eq!(result, ["minecraft:stick"]);
    }

    #[test]
    fn nested_members_preserve_order_deduplicate_and_do_not_truncate_at_sixteen() {
        let mut tags = std::collections::HashMap::new();
        for index in 0..64 {
            tags.insert(
                ("item".into(), format!("test:{index}")),
                vec![json!(format!("#test:{}", index + 1))],
            );
        }
        tags.insert(
            ("item".into(), "test:64".into()),
            vec![
                json!("minecraft:stick"),
                json!({"id":"minecraft:potion"}),
                json!("minecraft:stick"),
            ],
        );
        assert_eq!(
            members(&tags, "item", "test:0"),
            ["minecraft:stick", "minecraft:potion"]
        );
        assert!(members(&tags, "item", "test:missing").is_empty());
        tags.insert(("item".into(), "test:64".into()), vec![json!("#test:0")]);
        assert!(members(&tags, "item", "test:0").is_empty());
    }
}
