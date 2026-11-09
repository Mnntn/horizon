//! Command argument types mods register (`Registrar::register_command_argument`), kept for the
//! command trees built at every world open and refresh.
use super::*;
use std::sync::Arc;

/// A command argument type a mod registered, and the id a `Command` usage names.
#[derive(Clone)]
pub(crate) struct CommandArgumentInfo {
    pub(crate) id: Box<str>,
    pub(crate) argument: Arc<dyn crate::CommandArgumentType>,
}

/// The command argument types every installed mod registered, in registration order.
pub(crate) fn installed_command_arguments() -> &'static [CommandArgumentInfo] {
    installed().map_or(&[], |installed| installed.keys.command_arguments.as_slice())
}

impl dyn Registrar + '_ {
    /// Register command argument type `name` (`machine` or `mymod:machine`), once, from
    /// `Hooks::register`: a usage then names it as an argument's kind,
    /// `Command::new("machine use <machine:mymod:machine>")`, and the game parses and
    /// completes that argument with it. A name in another mod's namespace, an empty name, and an
    /// id registered twice refuse the mods at startup, naming the mod.
    pub fn register_command_argument<T: crate::CommandArgumentType>(
        &mut self,
        name: &str,
        argument: T,
    ) {
        self.add_command_argument(name, Arc::new(argument));
    }
}

/// Registrar operations for command argument types.
pub trait CommandArgumentsRegistrar {
    /// Register command argument type `name` with `argument`; mods reach it through
    /// `register_command_argument`.
    fn add_command_argument(&mut self, name: &str, argument: Arc<dyn crate::CommandArgumentType>);
}

impl CommandArgumentsRegistrar for PartRegistrar {
    fn add_command_argument(&mut self, name: &str, argument: Arc<dyn crate::CommandArgumentType>) {
        let (namespace, path) = name.split_once(':').unwrap_or((self.mod_id, name));
        let id = format!("{}:{}", self.mod_id, path);
        let refusal = if namespace != self.mod_id {
            Some(format!(
                "{} registered command argument type {name:?} in the namespace {namespace:?}, which is not its own",
                self.mod_id
            ))
        } else if path.is_empty() {
            Some(format!(
                "{} registered a command argument type with no name",
                self.mod_id
            ))
        } else {
            self.parts
                .command_arguments
                .iter()
                .any(|other| *other.id == id)
                .then(|| {
                    format!(
                        "{} registered command argument type {id} twice",
                        self.mod_id
                    )
                })
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.parts.command_arguments.push(CommandArgumentInfo {
            id: id.into(),
            argument,
        });
    }
}

/// Fixtures that tests of other crates use to install command argument types.
#[cfg(any(test, feature = "testing"))]
pub mod tests {

    #[test]
    pub(crate) fn registration_owns_its_namespace_and_refuses_duplicates() {
        /// A tiny argument with two valid values.
        struct Mode;

        impl crate::CommandArgumentType for Mode {
            fn kind(&self) -> crate::CommandArgumentKind {
                crate::CommandArgumentKind::Word
            }

            fn parse(&self, input: &str) -> Result<String, crate::CommandArgumentError> {
                match input {
                    "fast" | "slow" => Ok(input.to_owned()),
                    _ => Err(crate::CommandArgumentError::new(crate::Text::translate(
                        "test.commands.mode.unknown",
                        vec![crate::Text::literal(input)],
                    ))),
                }
            }

            fn suggest(&self, prefix: &str) -> Vec<crate::CommandSuggestion> {
                ["fast", "slow"]
                    .into_iter()
                    .filter(|mode| mode.starts_with(prefix))
                    .map(crate::CommandSuggestion::new)
                    .collect()
            }
        }

        struct Modes;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Modes {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_command_argument("mode", Mode);
            }
        }

        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_command_argument("mode", Mode);
                r.register_command_argument("mode", Mode);
                r.register_command_argument("other:mode", Mode);
                r.register_command_argument("", Mode);
            }
        }

        crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Modes),
        )])
        .unwrap();
        let installed = super::installed_command_arguments();
        assert_eq!(installed.len(), 1);
        assert_eq!(&*installed[0].id, "test:mode");
        assert_eq!(installed[0].argument.parse("fast"), Ok("fast".to_owned()));
        assert_eq!(
            installed[0].argument.suggest("s"),
            [crate::CommandSuggestion::new("slow")]
        );

        let errors = crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Invalid),
        )])
        .unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].contains("test registered command argument type test:mode twice"));
        assert!(errors[1].contains("namespace"));
        assert!(errors[2].contains("no name"));
    }
}
