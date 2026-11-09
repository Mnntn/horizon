use super::*;

/// Load-owned particle key requests, resolved before any world can emit particles.
pub trait ParticlesRegistrar {
    /// Request a vanilla particle or a custom sprite provider by canonical registry ID.
    /// Unknown IDs are reported with the requesting mod when the content registry loads.
    fn particle_key(&mut self, id: &str) -> crate::ParticleKey;
}
impl ParticlesRegistrar for PartRegistrar {
    fn particle_key(&mut self, id: &str) -> crate::ParticleKey {
        match self
            .registry_keys
            .borrow_mut()
            .particles
            .request(self.mod_id, id)
        {
            Ok(slot) => crate::ParticleKey(slot),
            Err(error) => {
                self.refused.push(error);
                crate::ParticleKey(u32::MAX)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hooks, ModEntry, RegistrationContext};
    use std::sync::Mutex;
    #[test]
    fn particle_registration_retains_requested_slots_until_atomic_publication() {
        struct Keys(Arc<Mutex<Vec<crate::ParticleKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Keys {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                let custom = registrar.particle_key("test:spark");
                let smoke = registrar.particle_key("smoke");
                assert_eq!(crate::ParticleKey::from_slot(custom.0), Ok(custom));
                assert_eq!(smoke, registrar.particle_key("minecraft:smoke"));
                self.0.lock().unwrap().extend([custom, smoke]);
            }
        }
        let keys = Arc::new(Mutex::new(Vec::new()));
        install_parts(
            &[ModEntry::new("keys", Box::new(Keys(keys.clone())))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        let keys = keys.lock().unwrap();
        assert!(
            crate::client::particle_keys::resolve_particles(["minecraft:smoke"], |_| Some(
                horizon_mod_runtime::registry_keys::ParticleKind::Custom
            ))
            .is_err()
        );
        assert!(!crate::client::particle_keys::particles_resolved());
        crate::client::particle_keys::resolve_particles(["minecraft:smoke", "test:spark"], |id| {
            Some(if id == "minecraft:smoke" {
                horizon_mod_runtime::registry_keys::ParticleKind::Vanilla(5)
            } else {
                horizon_mod_runtime::registry_keys::ParticleKind::Custom
            })
        })
        .unwrap();
        assert_eq!(
            crate::client::particle_keys::particle_name(keys[0]),
            Some("test:spark")
        );
        assert_eq!(
            crate::client::particle_keys::particle_vanilla_key(5),
            Some(keys[1])
        );
        assert_eq!(
            crate::client::particle_keys::particle_kind(keys[0]),
            Some(horizon_mod_runtime::registry_keys::ParticleKind::Custom)
        );
    }
}
