//! Opaque identities for fluid names retained from unavailable content.
use crate::{FluidKey, Host, UnresolvedFluid};
use std::{
    collections::HashMap,
    sync::{LazyLock, RwLock},
};

/// A registered fluid, or an opaque retained name from unavailable content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FluidIdentity {
    /// A registered fluid.
    Resolved(FluidKey),
    /// A name retained verbatim until its declaring mod returns.
    Unresolved(UnresolvedFluid),
}

impl FluidIdentity {
    /// Its registered key, or none when the fluid is unresolved.
    pub fn key(self) -> Option<FluidKey> {
        match self {
            Self::Resolved(key) => Some(key),
            Self::Unresolved(_) => None,
        }
    }
}

impl UnresolvedFluid {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u64) -> Result<Self, &'static str> {
        let table = TABLE.read().unwrap();
        (slot < table.names.len() as u64)
            .then_some(Self(slot))
            .ok_or("unknown unresolved fluid")
    }

    /// The original stored name, read from the host's process-lifetime table.
    pub fn id(self, host: &dyn Host) -> String {
        host.unresolved_fluid_id(self)
    }
}

#[derive(Default)]
struct Table {
    names: Vec<String>,
    slots: HashMap<String, u64>,
    utf16_slots: HashMap<Vec<u16>, u64>,
}
static TABLE: LazyLock<RwLock<Table>> = LazyLock::new(Default::default);
const MAX_IDS: usize = 4096;
const MAX_ID_BYTES: usize = 256;

/// Intern a stored name at an engine-owned data boundary.
pub fn intern(id: &str) -> Option<UnresolvedFluid> {
    let id = if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    };
    if id.len() > MAX_ID_BYTES || !namespaced_id(&id) || id.starts_with("minecraft:") {
        return None;
    }
    TABLE.write().unwrap().insert(id)
}

impl Table {
    fn insert(&mut self, id: String) -> Option<UnresolvedFluid> {
        if let Some(&slot) = self.slots.get(&id) {
            return Some(UnresolvedFluid(slot));
        }
        if self.names.len() >= MAX_IDS {
            return None;
        }
        let slot = self.names.len() as u64;
        self.names.push(id.clone());
        self.utf16_slots.insert(id.encode_utf16().collect(), slot);
        self.slots.insert(id, slot);
        Some(UnresolvedFluid(slot))
    }
}

/// Look up an already-interned identity without creating one.
pub fn lookup(id: &str) -> Option<UnresolvedFluid> {
    TABLE
        .read()
        .unwrap()
        .slots
        .get(id)
        .copied()
        .map(UnresolvedFluid)
}

/// Look up an already-interned NBT name without allocation.
pub fn lookup_utf16(id: &[u16]) -> Option<UnresolvedFluid> {
    TABLE
        .read()
        .unwrap()
        .utf16_slots
        .get(id)
        .copied()
        .map(UnresolvedFluid)
}

/// Compare an identity with an NBT name without allocating.
pub fn matches_utf16(identity: FluidIdentity, id: &[u16]) -> bool {
    match identity {
        FluidIdentity::Resolved(key) => {
            crate::content::registry_keys::fluid_key_utf16(id) == Some(key)
        }
        FluidIdentity::Unresolved(handle) => lookup_utf16(id) == Some(handle),
    }
}

/// Resolve an identity at an engine-owned boundary.
pub fn identity_name(identity: FluidIdentity) -> String {
    match identity {
        FluidIdentity::Resolved(key) => crate::content::registry_keys::fluid_id(key)
            .expect("resolved fluid key")
            .to_owned(),
        FluidIdentity::Unresolved(handle) => name(handle),
    }
}

/// Resolve an engine-issued process-lifetime handle.
pub fn name(fluid: UnresolvedFluid) -> String {
    TABLE.read().unwrap().names[fluid.0 as usize].clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_table_refuses_new_names_but_keeps_existing_ones() {
        let mut table = Table::default();
        for i in 0..MAX_IDS {
            assert!(table.insert(format!("test:fluid_{i}")).is_some());
        }
        assert!(table.insert("test:overflow".into()).is_none());
        assert!(table.insert("test:fluid_0".into()).is_some());
    }

    #[test]
    fn invalid_or_oversized_names_are_refused() {
        assert!(intern(&format!("test:{}", "x".repeat(257))).is_none());
        assert!(intern("test:bad space").is_none());
    }

    #[test]
    fn concurrent_intern_and_read_publish_complete_entries() {
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| intern("concurrency:fluid").unwrap());
            let reader = scope.spawn(|| {
                let id = "concurrency:fluid".encode_utf16().collect::<Vec<_>>();
                loop {
                    if let Some(handle) = lookup_utf16(&id) {
                        assert_eq!(name(handle), "concurrency:fluid");
                        break;
                    }
                    std::thread::yield_now();
                }
            });
            let handle = writer.join().unwrap();
            reader.join().unwrap();
            assert_eq!(
                lookup_utf16(&"concurrency:fluid".encode_utf16().collect::<Vec<_>>()),
                Some(handle)
            );
        });
    }

    #[test]
    fn handles_refuse_unissued_slots_and_survive_other_worlds() {
        assert_eq!(intern("mystery"), None);
        assert_eq!(intern("minecraft:mystery"), None);
        let old = intern("absent:fluid").unwrap();
        assert_eq!(Some(old), intern("absent:fluid"));
        assert!(UnresolvedFluid::from_slot(u64::MAX).is_err());
        let next = intern("other:fluid").unwrap();
        assert_ne!(old, next);
        assert_eq!(name(old), "absent:fluid");
    }
}

fn namespaced_id(id: &str) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b);
    id.split_once(':').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace.bytes().all(ok)
            && path.bytes().all(|b| ok(b) || b == b'/')
    })
}
#[cfg(test)]
mod identifier_tests {
    #[test]
    fn namespaced_identifier_samples() {
        for (id, expected) in [
            ("a:b/c", true),
            ("a_0.x-y:b/c.d-e_0", true),
            ("a:", false),
            (":b", false),
            ("a:b:c", false),
            ("A:b", false),
            ("a/b:c", false),
        ] {
            assert_eq!(super::namespaced_id(id), expected, "{id}");
        }
    }
}
