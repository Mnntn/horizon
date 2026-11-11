#[test]
fn entity_interact_reports_each_typed_hand() {
    struct Listener;
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_interact(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityInteractEvent<'_>,
        ) -> Flow {
            assert!(matches!(event.hand, Hand::Main | Hand::Off));
            if event.hand == Hand::Off {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    install(vec![ModEntry::new("hands", Box::new(Listener))].into());
    for hand in [Hand::Main, Hand::Off] {
        let mut claimed = false;
        dispatch_entity_interact(
            &mut TestHost,
            hand,
            42,
            "minecraft:zombie",
            None,
            &mut claimed,
        );
        assert_eq!(claimed, hand == Hand::Off);
    }
}
