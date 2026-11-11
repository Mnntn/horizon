use super::*;
thread_local! {
    static EXPECTED: std::cell::Cell<mod_api::Hand> = const { std::cell::Cell::new(mod_api::Hand::Main) };
    static RESULT: std::cell::Cell<mod_api::InteractionResult> = const { std::cell::Cell::new(mod_api::InteractionResult::Pass) };
    static SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
struct Part;
impl mod_api::ItemPart for Part {
    fn item_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[Some(mod_api::HookId::UseItem)]
    }
    fn use_item(
        &self,
        _: mod_api::HostSide<'_>,
        event: &mut mod_api::UseItemEvent<'_>,
    ) -> Option<mod_api::InteractionResult> {
        assert_eq!(event.hand, EXPECTED.get());
        SEEN.set(true);
        Some(RESULT.get())
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::Registrar) {
    let item = registrar.item_key("minecraft:stone");
    registrar.item_part(item, Part);
}
#[test]
fn generated_guest_item_dispatch_preserves_hands_sides_and_results() {
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    impl mod_api::ClientHost for World {}
    mod_api::impl_client_host_areas!(World);
    install_counter();
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap();
    for answer in [
        mod_api::InteractionResult::Success,
        mod_api::InteractionResult::SuccessServer,
        mod_api::InteractionResult::Consume,
        mod_api::InteractionResult::Pass,
        mod_api::InteractionResult::Fail,
        mod_api::InteractionResult::TryWithEmptyHand,
    ] {
        RESULT.set(answer);
        for client in [false, true] {
            for hand in [mod_api::Hand::Main, mod_api::Hand::Off] {
                EXPECTED.set(hand);
                SEEN.set(false);
                let mut result = None;
                let mut world = World;
                mod_api::dispatch_use_item(
                    if client {
                        mod_api::HostSide::Client(&mut world)
                    } else {
                        mod_api::HostSide::Server(&mut world)
                    },
                    mod_api::__private::UseItemCall {
                        stack: &stack,
                        hand,
                        result: &mut result,
                    },
                );
                assert!(SEEN.get());
                assert_eq!(result, Some(answer));
            }
        }
    }
}
