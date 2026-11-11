//! The tamed-death message hook: the dispatch a mod's answer goes through, with no game state.
use crate::mod_api::{self, Hooks, LivingDeathMessageEvent};
use horizon_client_core::text::Text;

/// Sets the owner's message to `text`.
struct Say(&'static str);
#[mod_api::__subscriptions]
impl Hooks for Say {
    fn living_death_message(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut LivingDeathMessageEvent<'_>,
    ) {
        *event.message = Some(mod_api::Text::literal(self.0));
    }
}

/// Tells the owner nothing.
struct Silence;
#[mod_api::__subscriptions]
impl Hooks for Silence {
    fn living_death_message(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut LivingDeathMessageEvent<'_>,
    ) {
        *event.message = None;
    }
}

/// Silences only the message another mod already wrote.
struct SilenceIf(&'static str);
#[mod_api::__subscriptions]
impl Hooks for SilenceIf {
    fn living_death_message(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut LivingDeathMessageEvent<'_>,
    ) {
        if event
            .message
            .as_ref()
            .is_some_and(|t| t == &mod_api::Text::literal(self.0))
        {
            *event.message = None;
        }
    }
}

fn owner_message(mods: Vec<Box<dyn Hooks>>) -> Option<String> {
    let mut message = Some(Text::literal("a wolf was slain"));
    let mods = mods
        .into_iter()
        .enumerate()
        .map(|(i, hooks)| mod_api::ModEntry::new(["first", "second"][i], hooks))
        .collect::<Vec<_>>()
        .into();
    mod_api::testing::with_mods(mods, || {
        let reader = horizon_client_render::api_context::LoopReader(
            crate::test_resources::server_loop_context(),
        );
        let context = reader.context();
        mod_api::dispatch_living_death_message(&context, 7, "minecraft:wolf", &mut message);
    });
    message.map(|t| t.plain())
}

#[test]
fn without_a_mod_the_owner_hears_the_built_message() {
    assert_eq!(owner_message(vec![]).as_deref(), Some("a wolf was slain"));
}

#[test]
fn a_mod_can_replace_the_owner_message() {
    let mods: Vec<Box<dyn Hooks>> = vec![Box::new(Say("the guard fell"))];
    assert_eq!(owner_message(mods).as_deref(), Some("the guard fell"));
}

#[test]
fn a_mod_can_silence_the_owner_message() {
    let mods: Vec<Box<dyn Hooks>> = vec![Box::new(Silence)];
    assert_eq!(owner_message(mods), None);
}

#[test]
fn mods_run_in_order_and_each_sees_the_answer_before_it() {
    let first_then_silence: Vec<Box<dyn Hooks>> =
        vec![Box::new(Say("first")), Box::new(SilenceIf("first"))];
    assert_eq!(owner_message(first_then_silence), None);
    let silence_then_first: Vec<Box<dyn Hooks>> =
        vec![Box::new(Silence), Box::new(SilenceIf("first"))];
    assert_eq!(owner_message(silence_then_first), None);
    let replace_then_keep: Vec<Box<dyn Hooks>> =
        vec![Box::new(Say("first")), Box::new(SilenceIf("other"))];
    assert_eq!(owner_message(replace_then_keep).as_deref(), Some("first"));
}
