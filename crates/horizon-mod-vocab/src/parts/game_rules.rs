use super::*;

impl PartRegistrar {
    pub(crate) fn game_rule_id(&self, id: &str) -> Result<String, crate::GameRuleError> {
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("{}:{id}", self.mod_id)
        };
        if crate::Identifier::parse_namespaced(&id).is_none() {
            return Err(crate::GameRuleError::InvalidId);
        }
        if !owns_namespace(self.mod_id, id.split_once(':').unwrap().0) {
            return Err(crate::GameRuleError::ForeignNamespace);
        }
        Ok(id)
    }
}

/// The immutable declarations installed after schema validation.
// hidden: the game builds engine registries from the installed declaration snapshot.
#[doc(hidden)]
pub fn game_rules() -> &'static [crate::GameRuleDecl] {
    installed().map_or(&[], |parts| &parts.game_rules)
}
/// The immutable category declarations installed by mods.
// hidden: the game builds engine registries from the installed category snapshot.
#[doc(hidden)]
pub fn game_rule_categories() -> &'static [String] {
    installed().map_or(&[], |parts| &parts.game_rule_categories)
}
/// Dense position for a validated rule key.
// hidden: the game resolves load-owned keys to dense positions.
#[doc(hidden)]
pub fn game_rule_bool_slot(key: crate::BoolGameRuleKey) -> Option<u32> {
    installed()?.game_rule_slots.get(key.0 as usize).copied()
}
/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;
    #[test]
    pub(crate) fn cross_mod_categories_resolve_after_all_registration() {
        use super::Registrar;
        struct Rule;
        #[mod_api::__subscriptions]
        impl Hooks for Rule {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_bool("active", "later:machines", true).unwrap();
            }
        }
        struct Category;
        #[mod_api::__subscriptions]
        impl Hooks for Category {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_category("machines").unwrap();
            }
        }
        let mods = [
            ModEntry::new("first", Box::new(Rule)),
            ModEntry::new("later", Box::new(Category)),
        ];
        super::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        assert!(
            super::game_rules()
                .iter()
                .any(|rule| rule.name == "first:active")
        );
    }
    #[test]
    pub(crate) fn unknown_category_reports_rule_category_and_owner_without_installing() {
        use super::Registrar;
        struct Rule;
        #[mod_api::__subscriptions]
        impl Hooks for Rule {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.game_rule_bool("active", "missing:machines", true)
                    .unwrap();
            }
        }
        let mods = [
            ModEntry::new("first", Box::new(Rule)),
            ModEntry::new("second", Box::new(Rule)),
        ];
        let errors = super::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
            .unwrap_err();
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("first:active"));
        assert!(errors[1].contains("second:active"));
        assert!(
            errors
                .iter()
                .all(|error| error.contains("missing:machines"))
        );
        assert!(
            !super::game_rules()
                .iter()
                .any(|rule| rule.name == "first:active")
        );
    }

    #[test]
    pub(crate) fn bool_rule_declarations_validate_slots_and_fail_atomically() {
        let mut refused = super::PartRegistrar::new("example");
        refused
            .game_rule_bool("active", "missing:machines", true)
            .unwrap();
        let _ = refused.game_rule_bool("foreign:bad", "example:machines", true);
        let errors = super::install_registrar(refused, &crate::parts::tests::rule_schema())
            .unwrap_err()
            .join(" ");
        assert!(
            errors.contains("ForeignNamespace")
                && errors.contains("example:active")
                && errors.contains("missing:machines")
        );
        let mut good = super::PartRegistrar::new("example");
        let _key = good
            .game_rule_bool("active", "example:machines", true)
            .unwrap();
        good.game_rule_category("machines").unwrap();
        assert_eq!(
            good.game_rule_bool("active", "example:machines", true),
            Err(crate::GameRuleError::Duplicate)
        );
        assert!(super::install_registrar(good, &crate::parts::tests::rule_schema()).is_err());
        let mut good = super::PartRegistrar::new("example");
        let key = good
            .game_rule_bool("active", "example:machines", true)
            .unwrap();
        good.game_rule_category("machines").unwrap();
        super::install_registrar(good, &crate::parts::tests::rule_schema()).unwrap();
        assert!(crate::BoolGameRuleKey::from_slot(key.0).is_ok());
        assert!(crate::BoolGameRuleKey::from_slot(key.0 + 1).is_err());
        assert!(super::game_rule_bool_slot(key).is_some());
    }

    #[test]
    pub(crate) fn category_registration_refuses_ignored_errors_and_installs_snapshot() {
        let mut foreign = super::PartRegistrar::new("example");
        assert_eq!(
            foreign.game_rule_category("foreign:machines"),
            Err(crate::GameRuleError::ForeignNamespace)
        );
        assert!(super::install_registrar(foreign, &crate::parts::tests::rule_schema()).is_err());
        let mut invalid = super::PartRegistrar::new("example");
        assert_eq!(
            invalid.game_rule_category("bad path"),
            Err(crate::GameRuleError::InvalidId)
        );
        assert!(super::install_registrar(invalid, &crate::parts::tests::rule_schema()).is_err());
        let mut good = super::PartRegistrar::new("example");
        assert_eq!(good.game_rule_category("machines"), Ok(()));
        assert!(super::install_registrar(good, &crate::parts::tests::rule_schema()).is_ok());
        assert!(super::game_rule_categories().contains(&"example:machines".to_owned()));
        let mut duplicate = super::PartRegistrar::new("example");
        duplicate.game_rule_category("machines").unwrap();
        assert_eq!(
            duplicate.game_rule_category("example:machines"),
            Err(crate::GameRuleError::Duplicate)
        );
        assert!(super::install_registrar(duplicate, &crate::parts::tests::rule_schema()).is_err());
    }
}

/// Registrar operations for game rules.
pub trait GameRulesRegistrar {
    /// Declare a game rule category in a namespace owned by the calling mod's manifest. Invalid,
    /// foreign or duplicate IDs refuse mod loading even if the error is ignored.
    /// Categories appear in the host's game rules screen; remote clients do not
    /// receive them in this version.
    fn game_rule_category(&mut self, id: &str) -> Result<(), crate::GameRuleError>;

    /// Declare a boolean game rule in a namespace owned by this mod's manifest. Category references are
    /// resolved after all mods register; an unknown category refuses installation then.
    fn game_rule_bool(
        &mut self,
        id: &str,
        category: &str,
        default: bool,
    ) -> Result<crate::BoolGameRuleKey, crate::GameRuleError>;
}

impl GameRulesRegistrar for PartRegistrar {
    fn game_rule_category(&mut self, id: &str) -> Result<(), crate::GameRuleError> {
        let result = (|| {
            let id = self.game_rule_id(id)?;
            if self.game_rule_categories.contains(&id) {
                return Err(crate::GameRuleError::Duplicate);
            }
            self.game_rule_categories.push(id);
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused
                .push(format!("mod {} category {id:?}: {error:?}", self.mod_id));
        }
        result
    }

    fn game_rule_bool(
        &mut self,
        id: &str,
        category: &str,
        default: bool,
    ) -> Result<crate::BoolGameRuleKey, crate::GameRuleError> {
        let result = (|| {
            let id = self.game_rule_id(id)?;
            if self.game_rules.iter().any(|r| r.name == id) {
                return Err(crate::GameRuleError::Duplicate);
            }
            let slot = self
                .registry_keys
                .borrow_mut()
                .game_rules
                .request(self.mod_id, &id)
                .map_err(|_| crate::GameRuleError::InvalidId)?;
            self.game_rules.push(crate::GameRuleDecl {
                name: id,
                category: category.into(),
                default,
            });
            Ok(crate::BoolGameRuleKey(slot))
        })();
        if let Err(error) = &result {
            self.refused
                .push(format!("mod {} rule {id:?}: {error:?}", self.mod_id));
        }
        result
    }
}
