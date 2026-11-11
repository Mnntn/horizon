use super::*;

/// The package inventory carries the same namespace set as content files and
/// the package loader's collision fence. A package always owns its own ID.
pub(crate) fn owns_namespace(owner: &str, namespace: &str) -> bool {
    owner == namespace
        || crate::LoadedMods::new()
            .get(owner)
            .is_some_and(|package| package.namespaces().any(|owned| owned == namespace))
}

pub(crate) fn owned_declaration_id(owner: &str, name: &str) -> Result<String, String> {
    let id = if name.contains(':') {
        name.to_owned()
    } else {
        format!("{owner}:{name}")
    };
    let parsed = crate::Identifier::parse_namespaced(&id)
        .ok_or_else(|| format!("mod {owner} declares invalid identifier {id:?}"))?;
    let namespace = parsed.namespace();
    if !owns_namespace(owner, namespace) {
        return Err(format!(
            "mod {owner} declares {id:?} in unowned namespace {namespace:?}"
        ));
    }
    Ok(id)
}

impl PartRegistrar {
    /// New identifiers only, not attachment targets: Parts may attach to
    /// vanilla or another mod's existing content.
    pub(crate) fn owned_declaration_id(&self, name: &str) -> Result<String, String> {
        owned_declaration_id(self.mod_id, name)
    }
}

impl crate::RegistrationContext {
    /// Refuse this mod's registration with a diagnostic, without panicking.
    /// The first reason wins. Reasons exceeding 4096 UTF-8 bytes are truncated
    /// at a character boundary; shorter reasons are preserved verbatim.
    /// All this mod's declarations from this registration are discarded, including
    /// later declarations, without adding their validation errors to the report.
    /// Other mods' declarations are unchanged. Native and wasm use the same
    /// admission path. Returning from `Hooks::register` immediately is recommended.
    pub fn refuse(&self, reason: &str) {
        let mut reason_slot = self.self_refusal.lock().unwrap_or_else(|p| p.into_inner());
        if reason_slot.is_none() {
            let mut end = reason.len().min(4096);
            while !reason.is_char_boundary(end) {
                end -= 1;
            }
            *reason_slot = Some(reason[..end].to_owned());
        }
    }

    pub(crate) fn refuse_declaration(&self, reason: String) {
        self.declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(reason);
    }
}

/// The engine consumes block/item declaration ownership refusals before loading
/// resources. Guest dispatch retains invalid output so the owning host boundary
/// sees and refuses it as well, rather than silently dropping a declaration.
// hidden: engine admission plumbing, re-exported through __private for the loader.
#[doc(hidden)]
pub fn take_declaration_refusals(context: &crate::RegistrationContext) -> Vec<String> {
    std::mem::take(
        &mut *context
            .declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    fn with_porter(f: impl FnOnce()) {
        crate::world::phase::with_test_package_inventory(
            vec![PackageInfo::new(
                "porter".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["porter".into(), "legacy".into()],
                vec![],
            )],
            f,
        );
    }

    #[test]
    fn manifest_owned_declarations_keep_the_persisted_namespace() {
        with_porter(|| {
            let mut r = PartRegistrar::new("porter");
            r.dimension_shape = |_| true;
            assert_eq!(r.owned_declaration_id("bare").unwrap(), "porter:bare");
            assert_eq!(r.owned_declaration_id("legacy:pot").unwrap(), "legacy:pot");
            let kind = r.declare_block_entity_type("legacy:pot", vec!["legacy:pot".into()]);
            assert_ne!(kind, BlockEntityTypeKey(u32::MAX));
            assert_eq!(r.block_entity_types[0].id, "legacy:pot");
            let fluid = r.declare_fluid("legacy:soup");
            assert_ne!(fluid, FluidKey(u32::MAX));
            assert!(
                r.registry_keys
                    .borrow()
                    .declared_fluids
                    .contains_key("legacy:soup")
            );
            let world_key = r.declare_world_fluid(
                "legacy:broth",
                WorldFluid {
                    tick_delay: 20,
                    drop_off: 2,
                    slope_find_distance: 3,
                    source_conversion: true,
                    bucket: false,
                },
            );
            assert_ne!(world_key, FluidKey(u32::MAX));
            assert_eq!(world_key, r.fluid_key("legacy:broth"));
            let keys = r.registry_keys.borrow();
            assert!(keys.declared_fluids.contains_key("legacy:broth"));
            assert!(keys.declared_fluids.contains_key("legacy:flowing_broth"));
            assert!(!keys.declared_fluids.contains_key("porter:broth"));
            drop(keys);
            r.declare_attribute("legacy:stamina", 1.0, 0.0, 10.0, true, Sentiment::Neutral)
                .unwrap();
            r.declare_entity_attributes("legacy:cook", "minecraft:zombie", vec![])
                .unwrap();
            r.add_dimension("legacy:kitchen", DimensionSettings::default());
            assert!(r.recipe_book_type("legacy:kitchen").is_ok());
            assert_eq!(r.attributes[0].id, "legacy:stamina");
            assert_eq!(r.entity_attributes[0].id, "legacy:cook");
            assert_eq!(&*r.dimensions[0].id, "legacy:kitchen");
            assert!(r.refused.is_empty(), "{:?}", r.refused);
            assert_eq!(
                r.declare_block_entity_type("unowned:pot", vec![]),
                BlockEntityTypeKey(u32::MAX)
            );
            let reason = r.refused.last().unwrap();
            assert!(
                reason.contains("porter") && reason.contains("unowned"),
                "{reason}"
            );
            assert!(r.owned_declaration_id("legacy:Bad Name").is_err());
        });
    }

    #[test]
    fn blocks_and_items_are_attributed_before_flattening_mod_outputs() {
        struct Declares;
        #[crate::__subscriptions]
        impl Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event.blocks.push(Block {
                    id: "legacy:pot".into(),
                    template: "minecraft:stone".into(),
                    ..Block::default()
                });
                event.blocks.push(Block {
                    id: "unowned:pot".into(),
                    template: "minecraft:stone".into(),
                    ..Block::default()
                });
            }
            fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
                event.items.push(Item {
                    id: "spoon".into(),
                    template: "minecraft:stick".into(),
                    ..Item::default()
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("porter", Box::new(Declares))].into();
        crate::testing::with_mods(mods, || {
            with_porter(|| {
                let context = RegistrationContext::new();
                let mut blocks = Vec::new();
                let mut items = Vec::new();
                crate::dispatch_blocks(&context, &mut blocks);
                crate::dispatch_items(&context, &mut items);
                assert_eq!(blocks[0].id, "legacy:pot");
                assert_eq!(items[0].id, "porter:spoon");
                // Invalid output stays visible for an outer host dispatch to refuse
                // too, rather than being silently lost inside a wasm guest.
                assert_eq!(blocks[1].id, "unowned:pot");
                let errors = take_declaration_refusals(&context);
                assert_eq!(errors.len(), 1);
                assert!(errors[0].contains("porter") && errors[0].contains("unowned"));
            })
        });
    }
}
