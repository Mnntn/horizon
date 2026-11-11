#!/usr/bin/env python3
"""Route declaration IDs through manifest ownership; run from the repo root.

The shared helper, dispatch ownership checks, and engine admission are reviewed
separately. Exact match counts make this mechanical pass fail on source drift.
"""
from pathlib import Path
import re


def replace(path, old, new, count=1):
    p = Path(path)
    text = p.read_text()
    assert text.count(old) == count, (path, old, text.count(old))
    p.write_text(text.replace(old, new))


parts = 'crates/horizon-mod-api/src/parts/'
for file, error in [('attributes.rs', 'AttributeRegistrationError'),
                    ('entity_attributes.rs', 'EntityAttributeError'),
                    ('spawn_placements.rs', 'SpawnPlacementError')]:
    replace(parts + file, '''            let (namespace, name) = path.split_once(':').unwrap_or((self.mod_id, path));
            if namespace.is_empty() || name.is_empty() || namespace != self.mod_id {
                return Err(crate::ERROR::InvalidId);
            }
'''.replace('ERROR', error), '''            let id = self.owned_declaration_id(path)
                .map_err(|reason| {
                    self.refused.push(reason);
                    crate::ERROR::InvalidId
                })?;
'''.replace('ERROR', error))
    replace(parts + file, '            let id = format!("{namespace}:{name}");\n', '')
replace(parts + 'attributes.rs', '''            // The characters modifier and catalog ids allow (`ResourceLocation`'s).
            let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
            if !namespace.bytes().all(valid) || !name.bytes().all(|c| valid(c) || c == b'/') {
                return Err(crate::AttributeRegistrationError::InvalidId);
            }
''', '')

# The registrations below have the same owner-only prefix and empty-name guard.
pattern = re.compile(r'''        let \(namespace, path\) = name.split_once\(':'.*?        let refusal = if namespace != self.mod_id \{.*?        \} else if path.is_empty\(\) \{.*?        \} else''', re.S)
for file, count in [('command_arguments.rs', 1), ('structures.rs', 1), ('dimensions.rs', 2)]:
    p = Path(parts + file)
    text = p.read_text()
    replacement = '''        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal ='''
    text, found = pattern.subn(replacement, text)
    assert found == count, (file, found)
    # The retained branch is either `if ...` or the duplicate-check block.
    p.write_text(text)

replace(parts + 'block_entities.rs', '        let id = format!("{}:{path}", self.mod_id);', '''        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return crate::BlockEntityTypeKey(u32::MAX);
            }
        };''')
replace(parts + 'fluids.rs', '''        let id = format!("{}:{path}", self.mod_id);
        if path.contains(':') || crate::Identifier::parse_namespaced(&id).is_none() {
            self.refused.push(format!(
                "mod {}: invalid fluid declaration {id:?}",
                self.mod_id
            ));
            return crate::FluidKey(u32::MAX);
        }''', '''        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return crate::FluidKey(u32::MAX);
            }
        };''')
replace(parts + 'fluids.rs', '''        let id = format!("{}:{path}", self.mod_id);
        let flowing = format!("{}:flowing_{path}", self.mod_id);''', '''        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return crate::FluidKey(u32::MAX);
            }
        };
        let (namespace, path) = id.split_once(':').expect("validated declaration id");
        let flowing = format!("{namespace}:flowing_{path}");''')

replace('crates/horizon-mod-package/src/content.rs', '''        let in_own =
            |id: &str| namespaced_id(id) && id.split_once(':').is_some_and(|(ns, _)| ns == own);''', '''        let in_own = |id: &str| namespaced_id(id)
            && id.split_once(':').is_some_and(|(ns, _)| self.namespaces.iter().any(|owned| owned == ns));''')
