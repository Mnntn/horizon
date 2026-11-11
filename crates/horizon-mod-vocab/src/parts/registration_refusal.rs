//! Self-refusal transport and the registration admission savepoint.
use super::*;

/// Engine admission plumbing; mods call `RegistrationContext::refuse` instead.
// hidden: the generated registrar call table transports self-refusals.
#[doc(hidden)]
pub trait RegistrationRefusalRegistrar {
    /// Deliver the current mod's refusal, without accepting an owner argument.
    fn registration_refusal(&mut self, reason: String);
}

/// Engine transport plumbing for the generated guest dispatch arm.
// hidden: drains the guest-local context after the registration callback.
#[doc(hidden)]
pub fn take_registration_refusal(cx: &crate::RegistrationContext) -> Option<String> {
    cx.self_refusal
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
}

impl RegistrationRefusalRegistrar for PartRegistrar {
    fn registration_refusal(&mut self, reason: String) {
        if self.self_refusal.is_none() {
            let context = crate::RegistrationContext::new();
            context.refuse(&reason);
            self.self_refusal = context
                .self_refusal
                .into_inner()
                .unwrap_or_else(|p| p.into_inner());
        }
    }
}

impl PartRegistrar {
    pub(crate) fn registration_checkpoint(&self) -> impl FnOnce(&mut Self) + use<> {
        let parts = self.parts.clone();
        let tags = self.tags.clone();
        let keys = self.registry_keys.borrow().clone();
        let menu_recipe_books = self.menu_recipe_books.clone();
        let recipe_book_types = self.recipe_book_types.clone();
        let registered = self.registered;
        // Append-only tables retain previous mods' entries without requiring
        // Clone on runtime callback objects or their eventual frozen caches.
        macro_rules! checkpoint {
            ($($field:ident),* $(,)?) => {{
                $(let $field = self.$field.len();)*
                move |registrar: &mut Self| {
                    $(registrar.$field.truncate($field);)*
                    registrar.parts = parts;
                    registrar.tags = tags;
                    *registrar.registry_keys.borrow_mut() = keys;
                    registrar.menu_recipe_books = menu_recipe_books;
                    registrar.recipe_book_types = recipe_book_types;
                    registrar.registered = registered;
                    registrar.self_refusal = None;
                }
            }};
        }
        checkpoint!(
            game_rule_categories,
            game_rules,
            block_entity_types,
            entity_attributes,
            world_fluids,
            entity_attribute_modifications,
            attributes,
            spawn_placements,
            controllers,
            arm_poses,
            structure_types,
            dimensions,
            chunk_generators,
            refused
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Callback;
    impl crate::BlockPart for Callback {
        fn block_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }
    impl crate::BlockEntityPart for Callback {
        fn block_entity_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }

    struct Refuses;
    impl Hooks for Refuses {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
            r.declare_fluid("before");
            let kind = r.block_entity_type("callback").blocks(&[]).build();
            r.block_entity_part(&kind, Callback).unwrap();
            cx.refuse("bootstrap failed: café\nmissing bridge");
            cx.refuse("ignored");
            r.declare_fluid("foreign:after");
        }
    }
    struct Healthy;
    impl Hooks for Healthy {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.declare_fluid("soup");
        }
    }

    #[test]
    fn refusal_rolls_back_only_its_owner_and_preserves_the_reason() {
        let mods = [
            ModEntry::new("healthy", Box::new(Healthy)),
            ModEntry::new("broken", Box::new(Refuses)),
            ModEntry::new("later", Box::new(Healthy)),
        ];
        let registrar = super::super::register_all(&mods, |_| false);
        assert_eq!(
            registrar.refused,
            ["mod broken refused registration: bootstrap failed: café\nmissing bridge"]
        );
        assert!(registrar.block_entity_types.is_empty());
        assert!(registrar.parts.block_entity_kinds.is_empty());
        let keys = registrar.registry_keys.borrow();
        assert!(keys.declared_fluids.contains_key("healthy:soup"));
        assert!(keys.declared_fluids.contains_key("later:soup"));
        assert!(!keys.declared_fluids.contains_key("broken:before"));
        assert!(!keys.declared_fluids.contains_key("foreign:after"));
    }

    #[test]
    fn bounded_reasons_preserve_utf8_and_empty_is_a_refusal() {
        for (reason, expected) in [
            ("a".repeat(4095) + "é!", "a".repeat(4095)),
            ("é".repeat(2048), "é".repeat(2048)),
            (String::new(), String::new()),
        ] {
            let cx = RegistrationContext::new();
            cx.refuse(&reason);
            cx.refuse("not the first reason");
            assert_eq!(*cx.self_refusal.lock().unwrap(), Some(expected));
        }
    }
}
