use super::*;
pub(crate) fn resolved_effects() -> &'static OnceLock<horizon_mod_runtime::registry_keys::Effects> {
    static EMPTY: OnceLock<horizon_mod_runtime::registry_keys::Effects> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.effects)
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn registry_keys_resolve_effects_declared_later_and_generated_name_reads() {
        struct Keys(Arc<std::sync::Mutex<Vec<EffectKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Keys {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let speed = r.effect_key("speed");
                let later = r.effect_key("key_test:later");
                assert_eq!(EffectKey::from_slot(speed.0), Ok(speed));
                assert_eq!(EffectKey::from_slot(later.0), Ok(later));
                assert!(EffectKey::from_slot(later.0 + 1).is_err());
                self.0.lock().unwrap().extend([speed, later]);
                assert_eq!(speed, r.effect_key("minecraft:speed"));
            }
        }
        let requested = Arc::new(std::sync::Mutex::new(Vec::new()));
        install_parts(
            &[
                ModEntry::new("first", Box::new(Keys(requested.clone()))),
                ModEntry::new("second", Box::new(Keys(requested.clone()))),
            ],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        let speed_key = requested.lock().unwrap()[0];
        assert_eq!(EffectKey::from_slot(speed_key.0), Ok(speed_key));
        assert!(EffectKey::from_slot(2).is_err());
        assert!(
            crate::content::registry_keys::effect_id(speed_key).is_none(),
            "startup has no resolved effects"
        );
        // Independent engine-index fixture: requested slots must not be confused
        // with the loaded registry indices, and unrequested effects are completed.
        let effects = [
            ("minecraft:speed", 2),
            ("key_test:later", 0),
            ("minecraft:poison", 1),
        ];
        crate::content::registry_keys::resolve_effects(effects.iter().map(|(id, _)| *id), |id| {
            effects
                .iter()
                .find(|(name, _)| *name == id)
                .map(|(_, index)| *index)
        })
        .unwrap();
        let keys = requested.lock().unwrap();
        assert_eq!(keys[0], keys[2]);
        assert_eq!(keys[1], keys[3]);
        assert_eq!(
            crate::content::registry_keys::effect_id(keys[0]),
            Some("minecraft:speed")
        );
        assert_eq!(
            crate::content::registry_keys::effect_id(keys[1]),
            Some("key_test:later")
        );
        assert_eq!(
            crate::content::registry_keys::effect_key_by_index(0),
            Some(keys[1])
        );
        assert_eq!(
            crate::content::registry_keys::effect_index(keys[0]),
            Some(2)
        );
        assert_eq!(
            crate::content::registry_keys::effect_index(keys[1]),
            Some(0)
        );
        for (id, index) in effects {
            let key = crate::content::registry_keys::effect_key_by_index(index).unwrap();
            assert_eq!(crate::content::registry_keys::effect_id(key), Some(id));
        }

        assert!(EffectKey::from_slot(u32::MAX).is_err());
        struct Table;
        impl mod_api::prelude::KeysBackend for Table {
            fn effect_key_id(&self, key: EffectKey) -> String {
                crate::content::registry_keys::effect_id(key)
                    .expect("invalid effect key slot")
                    .to_owned()
            }
        }
        impl Backend for Table {}
        mod_api::impl_backend_areas!(Table, except(keys));
        impl Host for Table {
            fn backend(&self) -> Option<BackendRef<'_>> {
                Some(BackendRef::of(self))
            }
        }
        mod_api::impl_host_areas!(Table);
        assert_eq!(keys[0].id(&Table), "minecraft:speed");
        assert_eq!(keys[1].id(&Table), "key_test:later");
        struct NoGame;
        impl Host for NoGame {}
        mod_api::impl_host_areas!(NoGame);
        assert!(std::panic::catch_unwind(|| keys[0].id(&NoGame)).is_err());
        assert_eq!(keys[0].category(&NoGame), None);
    }
}

/// Registrar operations for effects.
pub trait EffectsRegistrar {
    /// Request an effect by canonical registry id. All mods' declarations are checked at load.
    fn effect_key(&mut self, id: &str) -> crate::EffectKey;
}

impl EffectsRegistrar for PartRegistrar {
    fn effect_key(&mut self, id: &str) -> crate::EffectKey {
        match self
            .registry_keys
            .borrow_mut()
            .effects
            .request(self.mod_id, id)
        {
            Ok(slot) => crate::EffectKey(slot),
            Err(error) => {
                self.refused.push(error);
                crate::EffectKey(u32::MAX)
            }
        }
    }
}
