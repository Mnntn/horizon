use super::*;
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
use std::cell::{Cell, RefCell};
thread_local! { static LIVE: Cell<bool> = const { Cell::new(true) }; static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
fn matching(
    _: &horizon_world::native_load_context::LoopContext,
    kind: &str,
    _: &horizon_core::nbt::Compound,
    _: &ItemStack,
    accept: &mut bool,
) {
    if kind.starts_with("placement_guard:") {
        CALLS.with_borrow_mut(|calls| calls.push(kind.to_owned()));
        *accept = false;
        if kind == "placement_guard:changes_context" {
            LIVE.set(false);
        }
    }
}
#[test]
fn a_changed_context_interrupts_a_compound_before_the_next_custom_leaf() {
    assert!(crate::native_stack_hooks::install(
        crate::native_stack_hooks::StackHooks {
            ingredient_test: matching,
            enchantments: |_, _, _| {},
            attributes: |_, _| {}
        }
    ));
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let child = |kind: &str| {
        let mut value = Ingredient::of(vec!["fixture:item".into()]);
        value.custom = Some((kind.into(), Default::default()));
        value
    };
    let mut ingredient = Ingredient::of(vec!["fixture:item".into()]);
    ingredient.children = Some((
        Shape::Compound,
        vec![
            child("placement_guard:changes_context"),
            child("placement_guard:must_not_run"),
        ],
    ));
    assert_eq!(
        ingredient.test_in_guarded(&context, &ItemStack::new("fixture:item", 1), &|| LIVE.get()),
        None
    );
    CALLS.with_borrow(|calls| assert_eq!(calls, &["placement_guard:changes_context"]));
}
