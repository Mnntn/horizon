use super::*;
/// The ticket controllers registered at startup.
pub(crate) fn installed_controllers() -> &'static [ControllerInfo] {
    installed().map_or(&[], |installed| installed.controllers.as_slice())
}

/// The ticket controllers `mods` register, for a test's mods.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn registered_controllers(
    mods: &[ModEntry],
) -> (Vec<ControllerInfo>, Vec<crate::client::arm_pose::PoseInfo>) {
    let registered = register_all(mods, |_| false);
    (registered.controllers, registered.arm_poses)
}
/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;

    #[test]
    pub(crate) fn ticket_controllers_get_the_callers_namespace_and_refuse_anothers() {
        let mut r = super::PartRegistrar::new("first");
        r.register_ticket_controller("quarry", true);
        r.register_ticket_controller("first:drill", false);
        assert!(r.refused.is_empty(), "{:?}", r.refused);
        let ids: Vec<_> = r
            .controllers
            .iter()
            .map(|c| (&*c.id, c.validates))
            .collect();
        assert_eq!(ids, [("first:quarry", true), ("first:drill", false)]);
        // Another mod's namespace, and no name at all.
        r.register_ticket_controller("second:quarry", true);
        r.register_ticket_controller("first:", true);
        r.register_ticket_controller("", true);
        assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
        assert!(r.refused[0].contains("first declares \"second:quarry\""));
        assert!(r.refused[0].contains("unowned namespace \"second\""));
        assert!(
            r.refused[1].contains("invalid identifier")
                && r.refused[2].contains("invalid identifier")
        );
        assert_eq!(r.controllers.len(), 2);
    }

    #[test]
    fn installed_ticket_controllers_read_the_installed_table() {
        let mut registrar = PartRegistrar::new("first");
        registrar.register_ticket_controller("quarry", true);
        install_registrar(registrar, &crate::parts::tests::rule_schema()).unwrap();
        let controllers = installed_controllers();
        assert_eq!(controllers.len(), 1);
        assert_eq!(&*controllers[0].id, "first:quarry");
        assert!(controllers[0].validates);
    }

    #[test]
    pub(crate) fn a_ticket_controller_registered_twice_refuses_the_mods_naming_mod_and_id() {
        let mut r = super::PartRegistrar::new("first");
        r.register_ticket_controller("quarry", true);
        r.register_ticket_controller("first:quarry", false);
        r.mod_id = "second";
        // The same path under another mod is another id.
        r.register_ticket_controller("quarry", true);
        assert_eq!(r.refused.len(), 1, "{:?}", r.refused);
        assert_eq!(
            r.refused[0],
            "first registered ticket controller first:quarry twice"
        );
        assert_eq!(r.controllers.len(), 2);
        assert_eq!(&*r.controllers[1].id, "second:quarry");
    }
}

/// Registrar operations for tickets.
pub trait TicketsRegistrar {
    /// Register ticket controller `name`, the way NeoForge's `RegisterTicketControllersEvent`
    /// does once at startup, and keep the handle for `chunk.set_mod_forced`. `name` is
    /// `quarry` or `mymod:quarry`; the controller's id is in a namespace owned by the calling mod's manifest
    /// (`mymod:quarry`), and an unowned namespace refuses the mods at startup, as does an id
    /// registered twice. `validates` is whether it validates its saved tickets: the
    /// `validate_tickets` hook then runs for it when a level loads them (NeoForge's
    /// `LoadingValidationCallback`). A world's tickets of a controller no loaded mod registers are
    /// dropped when it loads.
    fn register_ticket_controller(&mut self, name: &str, validates: bool) -> TicketController;
}

impl TicketsRegistrar for PartRegistrar {
    fn register_ticket_controller(&mut self, name: &str, validates: bool) -> TicketController {
        ticket_controller::register(
            &mut self.controllers,
            &mut self.refused,
            self.mod_id,
            name,
            validates,
        )
    }
}
