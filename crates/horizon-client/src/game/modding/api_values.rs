// game-module-visibility: pub(crate)
//! The game-owned conversion boundary between API records and engine values.
//! Rust's orphan rule prevents foreign From/TryFrom implementations here.
use horizon_core::nbt::{Compound, JavaString, Tag};
use horizon_items::items::{ItemStack as EngineStack, Items};
use mod_api::item::item_stack::{ItemStack as ApiStack, StackWriteError};
#[path = "api_values/aggregates.rs"]
pub(in crate::game) mod aggregates;
#[path = "api_values/block_edit.rs"]
pub(in crate::game) mod block_edit;
#[path = "api_values/block_state.rs"]
pub(in crate::game) mod block_state;

pub(crate) fn block_jump_factor(
    context: &horizon_world::native_load_context::LoopContext,
    state: horizon_world::registry::BlockStateId,
    factor: &mut f32,
) {
    if !block_state::block_listens(mod_api::HookId::BlockJumpFactor, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_block_jump_factor(
        &horizon_client_render::api_context::LoopReader(context).context(),
        state,
        factor,
    );
}
pub(crate) fn block_speed_factor(
    context: &horizon_world::native_load_context::LoopContext,
    state: horizon_world::registry::BlockStateId,
    factor: &mut f32,
) {
    if !block_state::block_listens(mod_api::HookId::BlockSpeedFactor, state) {
        return;
    }
    let Ok(state) = state.try_into_api() else {
        return;
    };
    mod_api::dispatch_block_speed_factor(
        &horizon_client_render::api_context::LoopReader(context).context(),
        state,
        factor,
    );
}

/// Resolve the metadata default through the sole numeric reverse index, without allocation.
pub(crate) fn default_state(
    block: mod_api::BlockKey,
) -> Option<horizon_world::registry::BlockStateId> {
    use mod_api::BlockMetadata;
    let metadata = mod_api::__private::block_definitions()?;
    let value = mod_api::BlockState::from(metadata.of(block, &[]).ok()?);
    block_state::current().ok()?.to_engine(value).ok()
}

pub(crate) fn behavior_block(
    block: mod_api::BlockKey,
    behaviours: &horizon_world::level::Behaviours,
) -> Option<horizon_world::level::states::BlockId> {
    Some(behaviours.block(default_state(block)?))
}
pub(crate) use super::native_ground::alter_ground;
pub(in crate::game) use super::native_worldgen::prepare as prepare_block_states;
pub(in crate::game) use block_state::install as install_block_states;
#[path = "api_values/landing_write.rs"]
mod landing_write;
#[path = "api_values/recipe_inputs.rs"]
pub(in crate::game) mod recipe_inputs;

/// Publish immutable metadata with completed keys before any mesh tables or callbacks use it.
/// Registry definitions do not change on pack reload; world-number mappings are separate.
pub(in crate::game) fn resolve_block_definitions(
    registry: &horizon_world::registry::BlockRegistry,
) -> Result<(), String> {
    block_state::clear();
    if mod_api::__private::block_definitions().is_some() {
        return bind_worker_blocks(registry);
    }
    let mut blocks = std::collections::BTreeMap::<_, Vec<_>>::new();
    for index in 0..registry.len() {
        let state = registry
            .state(horizon_world::registry::BlockStateId(index as u32))
            .expect("registry index is in bounds");
        blocks
            .entry(state.name.as_str())
            .or_default()
            .push((index, &state.properties));
    }
    mod_api::__private::resolve_block_keys(blocks.keys().copied(), |name| {
        let states = &blocks[name];
        let default = registry
            .default_state(name)
            .expect("registry owns block defaults");
        let ordinal = states
            .iter()
            .position(|(index, _)| *index == default.0 as usize)
            .expect("default belongs to block");
        let states: Vec<std::collections::BTreeMap<_, _>> = states
            .iter()
            .map(|(_, properties)| {
                properties
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .collect();
        let states: Vec<_> = states.iter().collect();
        horizon_mod_runtime::block_definitions::BlockDefinition::new(&states, ordinal as u16)
            .map_err(|error| format!("block {name}: {error:?}"))
    })
    .map_err(|errors| errors.join("\n"))?;
    bind_worker_blocks(registry)
}

fn bind_worker_blocks(registry: &horizon_world::registry::BlockRegistry) -> Result<(), String> {
    block_state::bind(registry)?;
    let index = block_state::current().map_err(|error| format!("worker index: {error:?}"))?;
    let retained =
        super::native_worldgen::retain(registry, index).inspect_err(|_| block_state::clear())?;
    block_state::install(retained);
    Ok(())
}

#[cfg(test)]
#[test]
fn block_definition_import_preserves_defaults_and_is_immutable_after_load() {
    let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[
        ["air",true,false,"","minecraft:air"],
        ["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],
        ["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]
    ]}"#;
    mod_api::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let registry =
        horizon_world::registry::BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
    resolve_block_definitions(&registry).unwrap();
    let blocks = mod_api::__private::block_definitions().unwrap();
    let key = blocks.names.slots["minecraft:stairs"] as usize;
    assert_eq!(blocks.definitions[key].of(&[]), Ok(1));
    assert_eq!(
        blocks.definitions[key].property(0, "facing"),
        Ok(Some("north"))
    );
    assert_eq!(
        blocks.definitions[key].property(1, "facing"),
        Ok(Some("east"))
    );
    let changed = facts.replace("east", "south");
    let registry =
        horizon_world::registry::BlockRegistry::from_facts_json(changed.as_bytes()).unwrap();
    assert!(resolve_block_definitions(&registry).is_err());
    assert_eq!(
        mod_api::__private::block_definitions().unwrap().definitions[key].property(1, "facing"),
        Ok(Some("east"))
    );
}

impl TryIntoApi for horizon_client::boss_overlay::ModBar {
    type Api = mod_api::ClientBossBar;
    type Error = mod_api::text::TextFormError;

    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ClientBossBar {
            uuid: self.uuid,
            name: self.name.try_into_api()?,
            progress: self.progress,
            color: self.color,
            overlay: self.overlay,
            darken_screen: self.darken_screen,
            play_boss_music: self.play_boss_music,
            create_world_fog: self.create_world_fog,
        })
    }
}

/// Registry access for generated stack write-back in full and narrowed hook hosts.
pub(in crate::game) trait ItemBoundary {
    fn boundary_items(&self) -> Option<&Items>;
}
impl ItemBoundary for super::Game<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        Some(self.items())
    }
}
impl ItemBoundary for super::GameLevel<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        Some(self.items)
    }
}
impl ItemBoundary for super::native_hooks::HitLevel<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        self.0.tags()
    }
}

impl TryIntoApi for horizon_world::chunk::BlockState {
    type Api = mod_api::BlockState;
    type Error = mod_api::BlockStateError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let block = mod_api::__private::block_key(&self.name)
            .ok_or(mod_api::BlockStateError::UnknownBlock)?;
        let properties: Vec<_> = self
            .properties
            .iter()
            .map(|(name, value)| mod_api::StateConstraint::new(name.clone(), value.clone()))
            .collect();
        mod_api::BlockState::of(block_metadata_table()?, block, &properties)
    }
}

#[cfg(test)]
#[test]
fn block_values_keep_frozen_ordinals_when_world_state_ids_reorder() {
    use horizon_world::registry::{BlockRegistry, BlockStateId};
    mod_api::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let first = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["air",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]]}"#;
    let first = BlockRegistry::from_facts_json(first.as_bytes()).unwrap();
    resolve_block_definitions(&first).unwrap();
    let reordered = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["air",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}]]}"#;
    let reordered = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
    resolve_block_definitions(&reordered).unwrap();
    let east = reordered
        .state(BlockStateId(1))
        .unwrap()
        .try_into_api()
        .unwrap();
    let north = reordered
        .state(BlockStateId(2))
        .unwrap()
        .try_into_api()
        .unwrap();
    let metadata = block_metadata_table().unwrap();
    assert_eq!(east.ordinal(), 1);
    assert_eq!(north.ordinal(), 0);
    assert_eq!(east.property(metadata, "facing"), Ok(Some("east")));
    assert_eq!(north.property(metadata, "facing"), Ok(Some("north")));
    let mut invalid = reordered.state(BlockStateId(1)).unwrap().clone();
    invalid.properties.insert("facing".into(), "south".into());
    assert_eq!(
        invalid.try_into_api(),
        Err(mod_api::BlockStateError::InvalidValue(0))
    );
    invalid.name = "missing:block".into();
    assert_eq!(
        invalid.try_into_api(),
        Err(mod_api::BlockStateError::UnknownBlock)
    );
}

/// Prepare callback data before mesh construction; workers never convert engine records.
pub(in crate::game) fn mod_tint_values(
    _registry: &horizon_world::registry::BlockRegistry,
) -> Result<Option<crate::mesher::ModTintValues>, String> {
    if !crate::mesher::has_custom_block_tint_sources() {
        return Ok(None);
    }
    let metadata =
        block_metadata_table().map_err(|error| format!("block tint metadata: {error:?}"))?;
    let states = block_state::snapshot().map_err(|error| format!("block tint state: {error:?}"))?;
    Ok(Some(crate::mesher::ModTintValues { states, metadata }))
}

fn block_metadata_table()
-> Result<&'static horizon_mod_runtime::block_definitions::Blocks, mod_api::BlockStateError> {
    mod_api::__private::block_definitions().ok_or(mod_api::BlockStateError::Unavailable)
}
impl super::Game<'_> {
    pub(in crate::game) fn block_metadata_name(
        &self,
        block: mod_api::BlockKey,
    ) -> Result<&str, mod_api::BlockStateError> {
        mod_api::BlockMetadata::name(block_metadata_table()?, block)
    }
    pub(in crate::game) fn block_metadata_property(
        &self,
        state: mod_api::BlockStateRecord,
        name: &str,
    ) -> Result<Option<&str>, mod_api::BlockStateError> {
        mod_api::BlockMetadata::property(block_metadata_table()?, state, name)
    }
    pub(in crate::game) fn block_metadata_with(
        &self,
        state: mod_api::BlockStateRecord,
        name: &str,
        value: &str,
    ) -> Result<mod_api::BlockStateRecord, mod_api::BlockStateError> {
        mod_api::BlockMetadata::with(block_metadata_table()?, state, name, value)
    }
    pub(in crate::game) fn block_metadata_of(
        &self,
        block: mod_api::BlockKey,
        properties: &[mod_api::StateConstraint],
    ) -> Result<mod_api::BlockStateRecord, mod_api::BlockStateError> {
        mod_api::BlockMetadata::of(block_metadata_table()?, block, properties)
    }
}

/// Normalize an existing game write's answer without changing its implementation.
/// Generated API forwarders add the checked-input refusal around successful native answers.
pub(in crate::game) trait IntoWriteResult {
    type Value;
    type Error;
    fn into_write_result(self) -> Result<Self::Value, Self::Error>;
}
impl<T> IntoWriteResult for Option<T> {
    type Value = Self;
    type Error = StackWriteError;
    fn into_write_result(self) -> Result<Self, Self::Error> {
        Ok(self)
    }
}
impl<T, E> IntoWriteResult for Result<T, E> {
    type Value = T;
    type Error = E;
    fn into_write_result(self) -> Self {
        self
    }
}
macro_rules! write_answers {
    ($($ty:ty),* $(,)?) => { $(
        impl IntoWriteResult for $ty {
            type Value = Self;
            type Error = StackWriteError;
            fn into_write_result(self) -> Result<Self, Self::Error> { Ok(self) }
        }
    )* };
}
write_answers!(bool, u32, ());

/// Convert an engine value into an owned API value after resolving registry identities.
pub(crate) trait TryIntoApi {
    type Api;
    type Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error>;
}

/// Tests use the production snapshot conversion, never a second fixture codec.
#[cfg(test)]
pub(in crate::game) fn test_snapshot<T: TryIntoApi + ?Sized>(value: &T) -> T::Api
where
    T::Error: std::fmt::Debug,
{
    value
        .try_into_api()
        .expect("fixture snapshot has resolved item keys and a nonempty count")
}

/// Validate a mod value against the game's registry before admitting it to engine state.
#[path = "api_values/content_declarations.rs"]
mod content_declarations;

pub(crate) trait TryIntoEngine<Context: ?Sized = Items> {
    type Engine;
    type Error;
    fn try_into_engine(&self, context: &Context) -> Result<Self::Engine, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackReadError {
    UnknownItem,
    Empty,
    InvalidComponents(mod_api::StackError),
}

impl StackReadError {
    pub(in crate::game) fn into_write_error(self) -> StackWriteError {
        match self {
            Self::UnknownItem => StackWriteError::UnknownItem,
            Self::Empty => StackWriteError::EmptyEngineStack,
            Self::InvalidComponents(reason) => StackWriteError::InvalidComponents { reason },
        }
    }
}

/// Execute a native write and preserve its answer, including unit and original errors.
pub(in crate::game) fn call_write<W: IntoWriteResult>(
    call: impl FnOnce() -> W,
) -> Result<W::Value, W::Error> {
    call().into_write_result()
}

/// Convert a successful stack-bearing write answer through its original error channel.
pub(in crate::game) fn call_stack_write<W>(
    call: impl FnOnce() -> W,
) -> Result<<W::Value as TryIntoApi>::Api, W::Error>
where
    W: IntoWriteResult,
    W::Value: TryIntoApi<Error = StackReadError>,
    W::Error: From<StackWriteError>,
{
    call_write(call)?
        .try_into_api()
        .map_err(|error| error.into_write_error().into())
}

macro_rules! scalar_values {
    ($($ty:ty),* $(,)?) => { $(
        impl TryIntoApi for $ty {
            type Api = Self;
            type Error = StackReadError;
            fn try_into_api(&self) -> Result<Self, Self::Error> { Ok(*self) }
        }
        impl TryIntoEngine for $ty {
            type Engine = Self;
            type Error = StackWriteError;
            fn try_into_engine(&self, _: &Items) -> Result<Self, Self::Error> { Ok(*self) }
        }
    )* };
}
scalar_values!(u32, i32, bool, ());

impl TryIntoApi for String {
    type Api = Self;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self, Self::Error> {
        Ok(self.clone())
    }
}

impl<T: TryIntoApi + ?Sized> TryIntoApi for &T {
    type Api = T::Api;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        (*self).try_into_api()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C> + ?Sized> TryIntoEngine<C> for &T {
    type Engine = T::Engine;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        (*self).try_into_engine(context)
    }
}

impl<T: TryIntoApi> TryIntoApi for Option<T> {
    type Api = Option<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.as_ref().map(TryIntoApi::try_into_api).transpose()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for Option<T> {
    type Engine = Option<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.as_ref()
            .map(|value| value.try_into_engine(context))
            .transpose()
    }
}
impl<T: TryIntoApi, E: Clone> TryIntoApi for Result<T, E> {
    type Api = Result<T::Api, E>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        match self {
            Ok(value) => value.try_into_api().map(Ok),
            Err(error) => Ok(Err(error.clone())),
        }
    }
}

impl<T: TryIntoApi, const N: usize> TryIntoApi for [T; N] {
    type Api = [T::Api; N];
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut output: [Option<T::Api>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_api()?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>, const N: usize> TryIntoEngine<C> for [T; N] {
    type Engine = [T::Engine; N];
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        let mut output: [Option<T::Engine>; N] = std::array::from_fn(|_| None);
        for (target, source) in output.iter_mut().zip(self) {
            *target = Some(source.try_into_engine(context)?);
        }
        Ok(output.map(|value| value.expect("every array element converted")))
    }
}
impl<T: TryIntoApi> TryIntoApi for [T] {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<T: TryIntoApi> TryIntoApi for std::sync::Arc<[T]> {
    type Api = std::sync::Arc<[T::Api]>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(self.as_ref().try_into_api()?.into())
    }
}
impl<T: TryIntoApi> TryIntoApi for Vec<T> {
    type Api = Vec<T::Api>;
    type Error = T::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.iter().map(TryIntoApi::try_into_api).collect()
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for Vec<T> {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.as_slice().try_into_engine(context)
    }
}
impl<C: ?Sized, T: TryIntoEngine<C>> TryIntoEngine<C> for [T] {
    type Engine = Vec<T::Engine>;
    type Error = T::Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        self.iter()
            .map(|value| value.try_into_engine(context))
            .collect()
    }
}
impl TryIntoApi for (horizon_client::text::Text, u32) {
    type Api = (mod_api::Text, u32);
    type Error = mod_api::text::TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1))
    }
}
impl<C: ?Sized> TryIntoEngine<C> for (mod_api::Text, u32) {
    type Engine = (horizon_client::text::Text, u32);
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        Ok((self.0.try_into_engine(context)?, self.1))
    }
}

impl<A: TryIntoApi, B: TryIntoApi<Error = A::Error>> TryIntoApi for (A, B) {
    type Api = (A::Api, B::Api);
    type Error = A::Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok((self.0.try_into_api()?, self.1.try_into_api()?))
    }
}
impl<C: ?Sized> TryIntoEngine<C> for (mod_api::Text, mod_api::Text) {
    type Engine = (horizon_client::text::Text, horizon_client::text::Text);
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        Ok((
            self.0.try_into_engine(context)?,
            self.1.try_into_engine(context)?,
        ))
    }
}

impl TryIntoApi for super::spill::BlockItemDrop {
    type Api = mod_api::ItemDrop;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ItemDrop::new(
            self.stack.try_into_api()?,
            self.position,
            self.velocity,
            self.pickup_delay,
        ))
    }
}
impl TryIntoEngine for mod_api::ItemDrop {
    type Engine = super::spill::BlockItemDrop;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            random: None,
            stack: self.stack.try_into_engine(items)?,
            position: self.position,
            velocity: self.velocity,
            pickup_delay: self.pickup_delay,
        })
    }
}

impl TryIntoEngine for mod_api::ItemDisplayContext {
    type Engine = horizon_assets::item_model::DisplayContext;
    type Error = ();
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        use horizon_assets::item_model::DisplayContext as Engine;
        use mod_api::ItemDisplayContext as Api;
        Ok(match self {
            Api::None => Engine::None,
            Api::ThirdPersonLeftHand => Engine::ThirdPersonLeftHand,
            Api::ThirdPersonRightHand => Engine::ThirdPersonRightHand,
            Api::FirstPersonLeftHand => Engine::FirstPersonLeftHand,
            Api::FirstPersonRightHand => Engine::FirstPersonRightHand,
            Api::Head => Engine::Head,
            Api::Gui => Engine::Gui,
            Api::Ground => Engine::Ground,
            Api::Fixed => Engine::Fixed,
            Api::OnShelf => Engine::OnShelf,
            _ => return Err(()),
        })
    }
}

impl TryIntoApi for horizon_client::merchant::ItemCost {
    type Api = mod_api::ItemCost;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ItemCost::new(self.stack.try_into_api()?))
    }
}
impl TryIntoEngine for mod_api::ItemCost {
    type Engine = horizon_client::merchant::ItemCost;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            stack: self.stack.try_into_engine(items)?,
        })
    }
}
impl TryIntoApi for horizon_client::merchant::MerchantOffer {
    type Api = mod_api::MerchantOffer;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut offer = mod_api::MerchantOffer::new(
            self.base_cost_a.try_into_api()?,
            self.cost_b.try_into_api()?,
            self.result.try_into_api()?,
            self.max_uses,
            self.xp,
            self.price_multiplier,
        );
        offer.uses = self.uses;
        offer.reward_exp = self.reward_exp;
        offer.special_price_diff = self.special_price_diff;
        offer.demand = self.demand;
        Ok(offer)
    }
}
impl TryIntoEngine for mod_api::MerchantOffer {
    type Engine = horizon_client::merchant::MerchantOffer;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            base_cost_a: self.base_cost_a.try_into_engine(items)?,
            cost_b: self.cost_b.try_into_engine(items)?,
            result: self.result.try_into_engine(items)?,
            uses: self.uses,
            max_uses: self.max_uses,
            reward_exp: self.reward_exp,
            special_price_diff: self.special_price_diff,
            demand: self.demand,
            price_multiplier: self.price_multiplier,
            xp: self.xp,
        })
    }
}

impl TryIntoApi for EngineStack {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, Self::Error> {
        let key = mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?;
        let mut result = ApiStack::new(key, self.count).ok_or(StackReadError::Empty)?;
        if self.carries_component(None, "minecraft:damage") {
            result.set_damage(self.damage);
        }
        if let Some(Tag::Compound(patch)) = horizon_items::item_data::encode_extra_components(self)
        {
            result
                .set_components(patch)
                .map_err(StackReadError::InvalidComponents)?;
        }
        Ok(result)
    }
}

impl TryIntoApi for horizon_assets::item_model::StackView {
    type Api = ApiStack;
    type Error = StackReadError;

    fn try_into_api(&self) -> Result<ApiStack, StackReadError> {
        let count = u32::try_from(self.count)
            .ok()
            .filter(|&count| count > 0)
            .ok_or(StackReadError::Empty)?;
        let components = Tag::compound(self.patch.iter().map(|(name, value)| match value {
            Some(value) => (
                horizon_core::nbt::JavaString::from(name.as_str()),
                horizon_client_core::scoreboard::json_to_tag(value),
            ),
            None => (
                horizon_core::nbt::JavaString::from(format!("!{name}").as_str()),
                Tag::Compound(Compound::new()),
            ),
        }));
        let key = mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?;
        let mut stack = ApiStack::new(key, count).ok_or(StackReadError::Empty)?;
        let Tag::Compound(patch) = components else {
            unreachable!("model patch is constructed as a compound")
        };
        stack
            .set_components(patch)
            .map_err(StackReadError::InvalidComponents)?;
        Ok(stack)
    }
}

impl TryIntoEngine for mod_api::ItemResource {
    type Engine = EngineStack;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        self.as_stack().try_into_engine(items)
    }
}

/// Encode an API value for a read-only predicate or drawing, without imposing resident maxima.
/// Unknown keys and unrepresentable/invalid effect payloads still refuse conversion.
/// The write conversion below adds authoritative registry-dependent validation.
pub(in crate::game) fn stack_value(value: &ApiStack) -> Result<EngineStack, StackWriteError> {
    let id = mod_api::__private::item_key_id(value.item()).ok_or(StackWriteError::UnknownItem)?;
    encode_stack(value, id.into())
}

fn encode_stack(
    value: &ApiStack,
    item: std::sync::Arc<str>,
) -> Result<EngineStack, StackWriteError> {
    let mut patch = value.components().map(|patch| patch.as_nbt().clone());
    // Let the existing component codec preserve explicit zero versus prototype inheritance.
    if value.damage_component() == Some(0) {
        patch
            .get_or_insert_with(Default::default)
            .insert(JavaString::from("minecraft:damage"), Tag::Int(0));
    }
    let patch = patch.map(Tag::Compound);
    let (_, extra) = horizon_items::item_data::decode_components(patch.as_ref());
    let stack = EngineStack {
        item,
        count: value.count(),
        damage: value.damage(),
        extra,
    };
    content_declarations::validate_decoded_effects(patch.as_ref(), &stack)?;
    Ok(stack)
}

impl TryIntoEngine for ApiStack {
    type Engine = EngineStack;
    type Error = StackWriteError;

    fn try_into_engine(&self, items: &Items) -> Result<EngineStack, Self::Error> {
        let id =
            mod_api::__private::item_key_id(self.item()).ok_or(StackWriteError::UnknownItem)?;
        let (id, definition) = items
            .items
            .get_key_value(id)
            .ok_or(StackWriteError::UnknownItem)?;
        let stack = encode_stack(self, id.clone())?;
        let effective = |name: &str, default: u32| -> Result<u32, StackWriteError> {
            match stack.data().and_then(|data| data.other(name)) {
                Some(Tag::Int(value)) => {
                    u32::try_from(*value).map_err(|_| StackWriteError::InvalidMaximum)
                }
                Some(_) => Err(StackWriteError::InvalidMaximum),
                None => Ok(default),
            }
        };
        let maximum = if stack.carries_component(Some(definition), "minecraft:max_stack_size") {
            effective("minecraft:max_stack_size", definition.max_stack)?
        } else {
            1
        };
        if !(1..=99).contains(&maximum) {
            return Err(StackWriteError::InvalidMaximum);
        }
        if !horizon_world::item_template::durability_allows_stack_size(
            stack.carries_component(Some(definition), "minecraft:max_damage"),
            i64::from(maximum),
        ) {
            return Err(StackWriteError::StackableDamageable);
        }
        if stack.count > maximum {
            return Err(StackWriteError::CountExceeded { maximum });
        }
        let maximum = if stack.carries_component(Some(definition), "minecraft:max_damage") {
            effective("minecraft:max_damage", definition.max_damage)?
        } else {
            0
        };
        let damage = self.damage_component().map_or_else(
            || {
                u32::try_from(stack.effective_damage(Some(definition))).map_err(|_| {
                    StackWriteError::InvalidComponents {
                        reason: mod_api::StackError::InvalidDamage,
                    }
                })
            },
            Ok,
        )?;
        if damage > maximum {
            return Err(StackWriteError::DamageExceeded { maximum });
        }
        if i32::try_from(damage).is_err() {
            return Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage,
            });
        }
        Ok(stack)
    }
}

impl TryIntoApi for crate::crafting::RecipeRule {
    type Api = mod_api::RecipeRule;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Stonecutting { input, result } => mod_api::RecipeRule::Stonecutting {
                input: input.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTransform {
                template,
                base,
                addition,
                result,
            } => mod_api::RecipeRule::SmithingTransform {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::SmithingTrim {
                template,
                base,
                addition,
                pattern,
            } => mod_api::RecipeRule::SmithingTrim {
                template: template.try_into_api()?,
                base: base.try_into_api()?,
                addition: addition.try_into_api()?,
                pattern: pattern.try_into_api()?,
            },
            Self::BannerDuplicate { banner, result } => mod_api::RecipeRule::BannerDuplicate {
                banner: banner.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::ShieldDecoration {
                banner,
                target,
                result,
            } => mod_api::RecipeRule::ShieldDecoration {
                banner: banner.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Transmute {
                input,
                material,
                min,
                max,
                add_count,
                result,
            } => mod_api::RecipeRule::Transmute {
                input: input.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                add_count: *add_count,
                result: result.try_into_api()?,
            },
            Self::Dye {
                target,
                dye,
                result,
            } => mod_api::RecipeRule::Dye {
                target: target.try_into_api()?,
                dye: dye.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::Imbue {
                source,
                material,
                result,
            } => mod_api::RecipeRule::Imbue {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::BookCloning {
                source,
                material,
                min,
                max,
                result,
            } => mod_api::RecipeRule::BookCloning {
                source: source.try_into_api()?,
                material: material.try_into_api()?,
                min: *min,
                max: *max,
                result: result.try_into_api()?,
            },
            Self::FireworkRocket {
                shell,
                fuel,
                star,
                result,
            } => mod_api::RecipeRule::FireworkRocket {
                shell: shell.try_into_api()?,
                fuel: fuel.try_into_api()?,
                star: star.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                result,
            } => mod_api::RecipeRule::FireworkStar {
                dye: dye.try_into_api()?,
                fuel: fuel.try_into_api()?,
                trail: trail.try_into_api()?,
                twinkle: twinkle.try_into_api()?,
                shapes: shapes.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::FireworkStarFade {
                dye,
                target,
                result,
            } => mod_api::RecipeRule::FireworkStarFade {
                dye: dye.try_into_api()?,
                target: target.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::MapExtending {
                map,
                material,
                result,
            } => mod_api::RecipeRule::MapExtending {
                map: map.try_into_api()?,
                material: material.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::DecoratedPot {
                back,
                left,
                right,
                front,
                result,
            } => mod_api::RecipeRule::DecoratedPot {
                back: back.try_into_api()?,
                left: left.try_into_api()?,
                right: right.try_into_api()?,
                front: front.try_into_api()?,
                result: result.try_into_api()?,
            },
            Self::RepairItem => mod_api::RecipeRule::RepairItem,
        })
    }
}

impl TryIntoApi for crate::crafting::Ingredient {
    type Api = mod_api::Ingredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::Ingredient {
            items: self.items.clone(),
            order: self.order.clone(),
            custom: self.custom.clone(),
            children: self
                .children
                .as_ref()
                .map(|(shape, kids)| Ok((shape.clone(), kids.try_into_api()?)))
                .transpose()?,
        })
    }
}
impl TryIntoEngine for mod_api::Ingredient {
    type Engine = crate::crafting::Ingredient;
    type Error = StackWriteError;
    fn try_into_engine(&self, _items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::crafting::Ingredient {
            items: self.items.clone(),
            order: self.order.clone(),
            custom: self.custom.clone(),
            children: self
                .children
                .as_ref()
                .map(|(shape, kids)| {
                    Ok::<_, StackWriteError>((
                        shape.clone(),
                        kids.iter()
                            .map(|k| k.try_into_engine(_items))
                            .collect::<Result<_, _>>()?,
                    ))
                })
                .transpose()?,
        })
    }
}
impl TryIntoApi for crate::crafting::RecipeTemplate {
    type Api = mod_api::RecipeTemplate;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::RecipeTemplate::new(
            self.item.clone(),
            self.count,
            self.components.clone(),
        ))
    }
}
impl TryIntoApi for crate::crafting::RecipeTrimPattern {
    type Api = mod_api::RecipeTrimPattern;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Reference(id) => mod_api::RecipeTrimPattern::Reference(id.clone()),
            Self::Inline {
                asset_id,
                description,
                decal,
            } => mod_api::RecipeTrimPattern::Inline {
                asset_id: asset_id.clone(),
                description: description.clone(),
                decal: *decal,
            },
        })
    }
}
impl TryIntoApi for crate::crafting::FireworkShapeIngredient {
    type Api = mod_api::FireworkShapeIngredient;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::FireworkShapeIngredient::new(
            self.shape.try_into_api().unwrap(),
            self.ingredient.try_into_api()?,
        ))
    }
}
impl TryIntoApi for crate::crafting::RecipeSnapshot {
    type Api = mod_api::Recipe;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut result =
            mod_api::Recipe::new(self.id.clone(), self.kind.clone(), self.json.clone());
        result.group = self.group.clone();
        result.ingredients = self.ingredients.try_into_api()?;
        result.results = self.results.try_into_api()?;
        result.cooking_time = self.cooking_time;
        result.experience = self.experience;
        result.rule = self.rule.try_into_api()?;
        Ok(result)
    }
}

/// Install the engine-facing callbacks; snapshots never enter engine storage.
pub(crate) fn install_stack_hooks() {
    crate::native_stack_hooks::install(crate::native_stack_hooks::StackHooks {
        ingredient_test: |context, kind, data, stack, accept| {
            if !mod_api::__private::listened(mod_api::HookId::IngredientTest) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                horizon_client::crafting::native_hooks::native_ingredient_test(
                    context, kind, data, &stack, accept,
                );
            }
        },
        enchantments: |stack, target, value| {
            if !mod_api::__private::listened(mod_api::HookId::GetEnchantmentLevel) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                mod_api::item::enchantments::native_get_enchantment_level(&stack, target, value);
            }
        },
        attributes: |stack, value| {
            if !mod_api::__private::listened(mod_api::HookId::ItemAttributeModifier) {
                return;
            }
            if let Ok(stack) = stack.try_into_api() {
                mod_api::item::item_attributes::native_item_attribute_modifier(&stack, value);
            }
        },
    });
}

struct ApiSlot<'a> {
    engine: &'a mut dyn crate::menu::SlotView,
    invalid: bool,
}
impl mod_api::item::menu::SlotView for ApiSlot<'_> {
    fn item(&self) -> Option<ApiStack> {
        self.engine.item().and_then(|s| s.try_into_api().ok())
    }
    fn may_place(&self, stack: &ApiStack) -> bool {
        stack_value(stack).is_ok_and(|s| self.engine.may_place(&s))
    }
    fn may_pickup(&self) -> bool {
        self.engine.may_pickup()
    }
    fn allow_modification(&self) -> bool {
        self.engine.allow_modification()
    }
    fn safe_insert(&mut self, stack: Option<ApiStack>, count: u32) -> Option<ApiStack> {
        let value = match stack
            .as_ref()
            .map(|s| s.try_into_engine(self.engine.items()))
            .transpose()
        {
            Ok(s) => s,
            Err(_) => {
                self.invalid = true;
                return stack;
            }
        };
        match self.engine.safe_insert(value, count).try_into_api() {
            Ok(value) => value,
            Err(_) => {
                self.invalid = true;
                None
            }
        }
    }
    fn safe_take(&mut self, count: u32, max: u32) -> Option<ApiStack> {
        match self.engine.safe_take(count, max).try_into_api() {
            Ok(value) => value,
            Err(_) => {
                self.invalid = true;
                None
            }
        }
    }
}
/// Adapt a synchronous slot event; engine slot rules still own every write.
pub(crate) fn menu_slot_rules(
    context: &horizon_world::native_load_context::LoopContext,
    menu: &str,
    slot: usize,
    stack: &EngineStack,
    may_place: &mut bool,
    max_stack: &mut u32,
) {
    if !mod_api::__private::has_part_hook(mod_api::HookId::MenuSlotRules) {
        return;
    }
    let Ok(stack) = stack.try_into_api() else {
        return;
    };
    let reader = horizon_client_render::api_context::LoopReader(context);
    let context = reader.context();
    mod_api::dispatch_menu_slot_rules(&context, menu, slot, &stack, may_place, max_stack);
}

pub(crate) fn stacked_on_other(
    context: &horizon_world::native_load_context::LoopContext,
    event: &mut crate::menu::StackedOnOther<'_>,
    canceled: &mut bool,
) {
    if !mod_api::__private::listened(mod_api::HookId::ItemStackedOnOther) {
        return;
    }
    let Ok(mut carried) = event.carried.0.try_into_api() else {
        return;
    };
    let mut candidate_canceled = *canceled;
    let mut cancellation_result = event.cancellation_result;
    let mut admitted = None;
    let mut run = |clicked: &mut dyn crate::menu::SlotView| {
        let mut slot = ApiSlot {
            engine: clicked,
            invalid: false,
        };
        let mut api = mod_api::item::menu::StackedOnOther {
            player: event.player,
            player_entity_id: event.player_entity_id,
            slot: event.slot,
            container: event.container,
            container_slot: event.container_slot,
            primary: event.primary,
            carried: mod_api::item::menu::CarriedAccess(&mut carried),
            clicked: &mut slot,
            cancellation_result,
        };
        mod_api::dispatch_item_stacked_on_other(
            &horizon_client_render::api_context::LoopReader(context).context(),
            &mut api,
            &mut candidate_canceled,
        );
        cancellation_result = api.cancellation_result;
        if slot.invalid {
            return false;
        }
        let Ok(slot_value) = slot.engine.item().try_into_api() else {
            return false;
        };
        if slot_value.try_into_engine(slot.engine.items()).is_err() {
            return false;
        }
        match carried
            .as_ref()
            .map(|stack| stack.try_into_engine(slot.engine.items()))
            .transpose()
        {
            Ok(value) => {
                admitted = Some(value);
                true
            }
            Err(_) => false,
        }
    };
    let accepted = event.clicked.with_staged(&mut run);
    if accepted {
        *event.carried.0 = admitted.expect("an accepted cursor");
        event.cancellation_result = cancellation_result;
        *canceled = candidate_canceled;
    }
}

impl TryIntoApi for crate::stations::AnvilResult {
    type Api = mod_api::item::stations::AnvilResult;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::stations::AnvilResult {
            result: self.result.try_into_api()?,
            cost: self.cost,
            repair_item_count_cost: self.repair_item_count_cost,
            only_renaming: self.only_renaming,
        })
    }
}
impl TryIntoEngine for mod_api::item::stations::AnvilResult {
    type Engine = crate::stations::AnvilResult;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::stations::AnvilResult {
            result: self.result.try_into_engine(items)?,
            cost: self.cost,
            repair_item_count_cost: self.repair_item_count_cost,
            only_renaming: self.only_renaming,
        })
    }
}
impl TryIntoApi for crate::menu::AnvilCraft {
    type Api = mod_api::item::menu::AnvilCraft;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::menu::AnvilCraft {
            left: self.left.try_into_api()?,
            right: self.right.try_into_api()?,
            output: self.output.try_into_api()?,
        })
    }
}
impl TryIntoEngine for mod_api::item::menu::AnvilCraft {
    type Engine = crate::menu::AnvilCraft;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::AnvilCraft {
            left: self.left.try_into_engine(items)?,
            right: self.right.try_into_engine(items)?,
            output: self.output.try_into_engine(items)?,
        })
    }
}
impl TryIntoApi for crate::menu::GrindstonePlace {
    type Api = mod_api::item::menu::GrindstonePlace;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::menu::GrindstonePlace {
            top: self.top.try_into_api()?,
            bottom: self.bottom.try_into_api()?,
            vanilla_output: self.vanilla_output.try_into_api()?,
            output: self.output.try_into_api()?,
            xp: self.xp,
        })
    }
}
impl TryIntoEngine for mod_api::item::menu::GrindstonePlace {
    type Engine = crate::menu::GrindstonePlace;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::GrindstonePlace {
            top: self.top.try_into_engine(items)?,
            bottom: self.bottom.try_into_engine(items)?,
            vanilla_output: self.vanilla_output.try_into_engine(items)?,
            output: self.output.try_into_engine(items)?,
            xp: self.xp,
        })
    }
}
impl TryIntoApi for crate::menu::GrindstoneTake {
    type Api = mod_api::item::menu::GrindstoneTake;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::menu::GrindstoneTake {
            top: self.top.try_into_api()?,
            bottom: self.bottom.try_into_api()?,
            xp: self.xp,
            new_top: self.new_top.try_into_api()?,
            new_bottom: self.new_bottom.try_into_api()?,
        })
    }
}
impl TryIntoEngine for mod_api::item::menu::GrindstoneTake {
    type Engine = crate::menu::GrindstoneTake;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::GrindstoneTake {
            top: self.top.try_into_engine(items)?,
            bottom: self.bottom.try_into_engine(items)?,
            xp: self.xp,
            new_top: self.new_top.try_into_engine(items)?,
            new_bottom: self.new_bottom.try_into_engine(items)?,
        })
    }
}

impl TryIntoApi for crate::menu::AnvilUpdate {
    type Api = mod_api::item::menu::AnvilUpdate;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let vanilla = mod_api::item::stations::AnvilResult {
            result: self.vanilla.output.try_into_api()?,
            cost: self.vanilla.xp_cost,
            repair_item_count_cost: self.vanilla.material_cost.max(0) as u32,
            only_renaming: false,
        };
        let mut result = mod_api::item::menu::AnvilUpdate::new(
            self.left.try_into_api()?,
            self.right.try_into_api()?,
            self.name.clone(),
            &vanilla,
        );
        result.output = self.output.try_into_api()?;
        result.xp_cost = self.xp_cost;
        result.material_cost = self.material_cost;
        Ok(result)
    }
}

impl TryIntoEngine for mod_api::item::menu::AnvilUpdate {
    type Engine = crate::menu::AnvilUpdate;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::menu::AnvilUpdate {
            left: self.left.try_into_engine(items)?,
            right: self.right.try_into_engine(items)?,
            name: self.name.clone(),
            vanilla: crate::menu::AnvilVanilla {
                output: self.vanilla.output.try_into_engine(items)?,
                xp_cost: self.vanilla.xp_cost,
                material_cost: self.vanilla.material_cost,
            },
            output: self.output.try_into_engine(items)?,
            xp_cost: self.xp_cost,
            material_cost: self.material_cost,
        })
    }
}
impl super::Game<'_> {
    pub(in crate::game) fn api_anvil_update(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::AnvilUpdate,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        let original = api.clone();
        self.native_anvil_update(player, entity, pos, &mut api, cancel);
        if api == original {
            return;
        }
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_grindstone_place(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::GrindstonePlace,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        let original = api.clone();
        self.native_grindstone_place(player, entity, pos, &mut api, cancel);
        if api == original {
            return;
        }
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_grindstone_take(
        &mut self,
        player: u128,
        entity: i32,
        pos: [i32; 3],
        value: &mut crate::menu::GrindstoneTake,
        cancel: &mut bool,
    ) {
        let Ok(mut api) = value.try_into_api() else {
            return;
        };
        let original = api.clone();
        self.native_grindstone_take(player, entity, pos, &mut api, cancel);
        if api == original {
            return;
        }
        match api.try_into_engine(self.items()) {
            Ok(result) => *value = result,
            Err(_) => *cancel = true,
        }
    }
    pub(in crate::game) fn api_anvil_craft_pre(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        value: &crate::menu::AnvilCraft,
        cancel: &mut bool,
    ) {
        if let Ok(api) = value.try_into_api() {
            self.native_anvil_craft_pre(player, name, pos, &api, cancel);
        }
    }
    pub(in crate::game) fn api_anvil_craft_post(
        &mut self,
        player: u128,
        name: &str,
        pos: [i32; 3],
        value: &crate::menu::AnvilCraft,
    ) {
        if let Ok(api) = value.try_into_api() {
            self.native_anvil_craft_post(player, name, pos, &api);
        }
    }
}

#[cfg(test)]
mod attribute_boundary_tests {
    use super::EngineStack as ItemStack;
    use crate::item_attributes::attribute_modifiers_or;
    use horizon_items::item_data::ModifierRow;
    use mod_api::{
        AttributeModifier, AttributeOperation, EquipmentSlotGroup, Hooks, Identifier,
        ItemAttributeModifierEvent, ModEntry, testing::with_mods,
    };
    use std::borrow::Cow;
    /// Makes a netherite chestplate heavier: +0.4 knockback resistance in
    /// the chest slot, and no armor toughness.
    struct Heavy;
    #[mod_api::__subscriptions]
    impl Hooks for Heavy {
        fn item_attribute_modifier(&self, event: &mut ItemAttributeModifierEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item())
                != Some("minecraft:netherite_chestplate")
            {
                return;
            }
            let id = |text: &str| Identifier::parse(text).unwrap();
            let heavy = |amount| {
                AttributeModifier::new(id("test:heavy"), amount, AttributeOperation::AddValue)
            };
            let resistance = id("minecraft:knockback_resistance");
            assert!(
                event
                    .modifiers
                    .add(resistance.clone(), heavy(0.4), EquipmentSlotGroup::Chest)
            );
            // The id is taken on that attribute.
            assert!(
                !event
                    .modifiers
                    .add(resistance, heavy(1.0), EquipmentSlotGroup::Chest)
            );
            assert!(
                event
                    .modifiers
                    .remove_all_for(&id("minecraft:armor_toughness"))
            );
        }
    }

    fn chestplate() -> Vec<ModifierRow> {
        ["minecraft:armor", "minecraft:armor_toughness"]
            .map(|attribute| {
                ModifierRow::new(
                    attribute,
                    "minecraft:armor.chestplate",
                    3.0,
                    "add_value",
                    "chest",
                )
            })
            .to_vec()
    }

    /// A mod's edits are what the stack grants; a stack it leaves alone, and
    /// any stack with no mod listening, keeps the item's list borrowed.
    #[test]
    fn a_mod_edits_what_a_stack_grants() {
        super::prepare_stack_boundary_tests();
        let types = chestplate();
        let netherite = ItemStack::new("minecraft:netherite_chestplate", 1);
        let diamond = ItemStack::new("minecraft:diamond_chestplate", 1);
        with_mods(Vec::new().into(), || {
            assert!(matches!(
                attribute_modifiers_or(&netherite, &types),
                Cow::Borrowed(_)
            ));
        });
        with_mods(vec![ModEntry::new("heavy", Box::new(Heavy))].into(), || {
            assert_eq!(
                &*attribute_modifiers_or(&netherite, &types),
                [
                    types[0].clone(),
                    ModifierRow::new(
                        "minecraft:knockback_resistance",
                        "test:heavy",
                        0.4,
                        "add_value",
                        "chest"
                    ),
                ]
            );
            assert!(matches!(
                attribute_modifiers_or(&diamond, &types),
                Cow::Borrowed(_)
            ));
        });
    }

    /// A chestplate a server sent with modifiers of its own, worn for the first
    /// time: the first equipment pass (`collectEquipmentChanges` with nothing
    /// worn before) asks the mods about the decoded list, so the map's first
    /// values are the mod's answer.
    #[test]
    fn a_servers_stack_takes_the_mods_answer_on_first_equip() {
        use horizon_client::living_attributes::LivingAttributes;
        use horizon_items::equipment::{CHEST, Equipment};
        use horizon_protocol::buf::{Codec, Reader, Writer};
        use horizon_protocol::item::{Slot, component};
        use horizon_protocol::registry::{Builtin, Dynamic, Ids};
        use horizon_world::{attribute_map::ValueKind, attributes::entity::Catalog};
        use std::sync::Arc;
        super::prepare_stack_boundary_tests();
        let attributes = [
            "minecraft:armor",
            "minecraft:armor_toughness",
            "minecraft:knockback_resistance",
        ];
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut components = vec![String::new(); 17];
        components[component::ATTRIBUTE_MODIFIERS as usize] =
            "minecraft:attribute_modifiers".into();
        let builtin = Builtin {
            registries: [
                (
                    "minecraft:item".to_owned(),
                    Ids::from_names(names(&["minecraft:air", "minecraft:netherite_chestplate"])),
                ),
                (
                    "minecraft:data_component_type".to_owned(),
                    Ids::from_names(components),
                ),
                (
                    "minecraft:attribute".to_owned(),
                    Ids::from_names(names(&attributes)),
                ),
            ]
            .into(),
        };
        // The server's patch: 10 armor and 4 toughness in the chest slot group (6).
        let mut wire = Writer::new();
        for v in [1, 1, 1, 0, component::ATTRIBUTE_MODIFIERS, 2] {
            wire.varint(v);
        }
        for (attribute, amount) in [(0, 10.0), (1, 4.0)] {
            wire.varint(attribute);
            wire.string("test:forged");
            wire.f64(amount);
            for v in [0, 6, 0] {
                wire.varint(v);
            }
        }
        let sent = Slot::read(&mut Reader::new(&wire.data)).unwrap().unwrap();
        let chestplate =
            horizon_client::item_sync::from_network(&sent, &builtin, &Dynamic::default());
        let bits = |n: f64| format!("{:016x}", n.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": horizon_world::DATA_VERSION,
                "attributes": attributes.map(|id| serde_json::json!({
                    "id": id, "default": bits(0.0), "min": bits(0.0), "max": bits(30.0),
                    "syncable": true
                })),
                "entities": {
                    "minecraft:armor_stand": serde_json::Map::from_iter(
                        attributes.map(|id| (id.to_owned(), serde_json::json!(bits(0.0))))
                    )
                }
            }))
            .unwrap(),
        );
        let items = super::stack_limit_tests::catalog();
        let mut equipment = Equipment::default();
        equipment.set(CHEST, Some(chestplate));
        with_mods(vec![ModEntry::new("heavy", Box::new(Heavy))].into(), || {
            let mut live = LivingAttributes::default();
            live.collect_equipment_changes(
                &catalog,
                "minecraft:armor_stand",
                &equipment,
                &items,
                Vec::new,
            )
            .unwrap();
            let value = |name| {
                live.read(
                    &catalog,
                    "minecraft:armor_stand",
                    name,
                    ValueKind::Computed,
                    Vec::new,
                )
            };
            // The server's armor, the mod's knockback resistance, no toughness.
            assert_eq!(value("minecraft:armor"), Some(10.0));
            assert_eq!(value("minecraft:knockback_resistance"), Some(0.4));
            assert_eq!(value("minecraft:armor_toughness"), Some(0.0));
        });
    }
}

#[cfg(test)]
mod enchantment_boundary_tests {
    use super::EngineStack as ItemStack;
    use crate::enchantment_levels::get_enchantment_level;
    use mod_api::{GetEnchantmentLevelEvent, Hooks, Identifier, ModEntry, testing::with_mods};
    use std::borrow::Cow;
    use std::sync::Arc;
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }
    /// An iron sword fights as if it had Looting II and no Sharpness.
    struct Keen;
    #[mod_api::__subscriptions]
    impl Hooks for Keen {
        fn get_enchantment_level(&self, event: &mut GetEnchantmentLevelEvent<'_>) {
            if mod_api::__private::item_key_id(event.stack.item()) != Some("minecraft:iron_sword") {
                return;
            }
            let looting = id("minecraft:looting");
            if event.target.is_none_or(|target| *target == looting) {
                event.enchantments.upgrade(looting, 2);
            }
            event.enchantments.set(id("minecraft:sharpness"), 0);
        }
    }

    /// Gameplay reads see the mod's levels; the stack's own component, a
    /// stack the mod leaves alone and every stack with no mod listening
    /// keep theirs, borrowed.
    #[test]
    fn a_mod_changes_the_levels_gameplay_reads() {
        super::prepare_stack_boundary_tests();
        let _ = horizon_items::native_enchantment_hooks::install(get_enchantment_level);
        let mut iron = ItemStack::new("minecraft:iron_sword", 1);
        iron.enchant("minecraft:sharpness", 3);
        let mut diamond = ItemStack::new("minecraft:diamond_sword", 1);
        diamond.enchant("minecraft:sharpness", 3);
        with_mods(Vec::new().into(), || {
            assert!(matches!(iron.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 3);
        });
        with_mods(vec![ModEntry::new("keen", Box::new(Keen))].into(), || {
            assert_eq!(
                &*iron.all_enchantments(),
                [(Arc::from("minecraft:looting"), 2)]
            );
            assert_eq!(iron.enchantment_level("minecraft:looting"), 2);
            assert_eq!(iron.enchantment_level("minecraft:sharpness"), 0);
            assert_eq!(iron.tag_enchantment_level("minecraft:sharpness"), 3);
            assert_eq!(iron.enchantments(), [(Arc::from("minecraft:sharpness"), 3)]);
            assert!(matches!(diamond.all_enchantments(), Cow::Borrowed(_)));
            assert_eq!(diamond.enchantment_level("minecraft:sharpness"), 3);
        });
    }
}

#[cfg(test)]
fn prepare_stack_boundary_tests() {
    install_stack_hooks();
    install_parts(&[]).unwrap();
    mod_api::__private::resolve_items([
        "minecraft:iron_sword",
        "minecraft:stick",
        "minecraft:diamond_sword",
        "minecraft:netherite_chestplate",
        "minecraft:diamond_chestplate",
        "minecraft:potion",
    ])
    .unwrap();
}

#[cfg(test)]
mod brewed_boundary_tests {
    use super::*;
    struct NoHost;
    impl mod_api::Host for NoHost {}
    mod_api::impl_host_areas!(NoHost);
    impl mod_api::ServerHost for NoHost {}
    mod_api::impl_server_host_areas!(NoHost);
    struct Watch(std::sync::Arc<std::sync::Mutex<Vec<(u128, i32, String, u32)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Watch {
        fn player_brewed_potion(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerBrewedPotionEvent<'_>,
        ) {
            self.0.lock().unwrap().push((
                event.player,
                event.player_entity_id,
                mod_api::__private::item_key_id(event.stack.item())
                    .unwrap()
                    .to_owned(),
                event.stack.count(),
            ));
        }
    }
    #[test]
    fn brewed_event_reports_player_and_converted_stack() {
        prepare_stack_boundary_tests();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "watch",
                Box::new(Watch(seen.clone())),
            )]
            .into(),
            || {
                let stack = EngineStack::new("minecraft:potion", 1)
                    .try_into_api()
                    .unwrap();
                mod_api::dispatch_player_brewed_potion(&mut NoHost, 7, 41, &stack);
            },
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [(7, 41, "minecraft:potion".to_owned(), 1)]
        );
    }
}

impl TryIntoApi for crate::containers::ModBlockEntityFields {
    type Api = mod_api::item::containers::ModBlockEntityFields;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::item::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_api()?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoEngine for mod_api::item::containers::ModBlockEntityFields {
    type Engine = crate::containers::ModBlockEntityFields;
    type Error = StackWriteError;
    fn try_into_engine(&self, items: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(crate::containers::ModBlockEntityFields {
            nbt: self.nbt.clone(),
            items: self.items.try_into_engine(items)?,
            data: self.data.clone(),
        })
    }
}
impl TryIntoApi for crate::containers::ModBlockEntity {
    type Api = mod_api::ModBlockEntity;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(mod_api::ModBlockEntity::from_snapshot(
            self.id().to_owned(),
            (**self).try_into_api()?,
            mod_api::item::containers::ModRevision::from_snapshot(
                self.revision().identity(),
                self.revision().changes(),
            ),
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BlockEntitySnapshotError {
    InvalidRevision,
    InvalidStack,
}

fn apply_block_entity_snapshot(
    entity: &mut crate::containers::ModBlockEntity,
    snapshot: &mod_api::ModBlockEntity,
    before: mod_api::item::containers::ModRevision,
    items: &Items,
) -> Result<(), BlockEntitySnapshotError> {
    let current = entity.revision();
    let revision = snapshot.revision();
    if snapshot.id() != entity.id()
        || revision.identity() != current.identity()
        || revision.changes() < current.changes()
    {
        return Err(BlockEntitySnapshotError::InvalidRevision);
    }
    if revision != before {
        let fields = (**snapshot)
            .try_into_engine(items)
            .map_err(|_| BlockEntitySnapshotError::InvalidStack)?;
        entity.apply_snapshot(fields, snapshot.revision().changes());
    }
    if let Some(fields) = snapshot.client_update_fields() {
        entity.set_client_update_fields(fields.clone());
    }
    Ok(())
}

// Validation runs after all listeners return; dispatch does not retain per-mod provenance.
// Bound diagnostics even when mods register many entity types.
fn finish_block_entity_writeback<T>(
    entity: &mut crate::containers::ModBlockEntity,
    snapshot: &mod_api::ModBlockEntity,
    before: mod_api::item::containers::ModRevision,
    items: &Items,
    hook: &'static str,
    output: Result<T, BlockEntitySnapshotError>,
) -> Result<T, BlockEntitySnapshotError> {
    let result = output.and_then(|output| {
        apply_block_entity_snapshot(entity, snapshot, before, items)?;
        Ok(output)
    });
    if let Err(reason) = &result {
        let changes = entity
            .revision()
            .changes()
            .max(snapshot.revision().changes())
            .saturating_add(1);
        entity.apply_snapshot((**entity).clone(), changes);
        static REPORTED: std::sync::Mutex<Vec<(String, &'static str)>> =
            std::sync::Mutex::new(Vec::new());
        let mut reported = REPORTED.lock().unwrap_or_else(|poison| poison.into_inner());
        if reported.len() < 1024
            && !reported
                .iter()
                .any(|(id, name)| id == entity.id() && *name == hook)
        {
            reported.push((entity.id().to_owned(), hook));
            log::warn!(
                "Rejected block entity writeback: entity type {}, hook {}, reason {:?}",
                entity.id(),
                hook,
                reason
            );
        }
    }
    result
}

impl super::Game<'_> {
    pub(in crate::game) fn api_block_entity_tick(
        &mut self,
        pos: [i32; 3],
        entity: &mut crate::containers::ModBlockEntity,
    ) {
        // #2051: the one decision whether a block entity ticks is its
        // type's own ticker (`native_block_entity_ticks`,
        // `EntityBlock.getTicker`), answered when the world opens — the
        // level's ticking list carries it here, whichever path registered
        // the type (a Registrar part or a bare declaration whose tick
        // arrives another way). No second gate: a parts-only check here
        // kept declared types' ticks — and the injections on
        // `native_block_entity_tick` — unreachable.
        let Ok(mut api) = entity.try_into_api() else {
            return;
        };
        let revision = api.revision();
        self.native_block_entity_tick(pos, &mut api);
        let _ = finish_block_entity_writeback(
            entity,
            &api,
            revision,
            self.items(),
            "block_entity_tick",
            Ok(()),
        );
    }
    pub(in crate::game) fn api_block_entity_update_tag(
        pos: [i32; 3],
        entity: &crate::containers::ModBlockEntity,
        tag: &mut Option<horizon_core::nbt::Compound>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityUpdateTag) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            Self::native_block_entity_update_tag(pos, &api, tag);
        }
    }
    pub(in crate::game) fn api_block_entity_submit(
        &self,
        pos: [i32; 3],
        block: &str,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        draw: &mut super::native_block_entity_draw::BlockEntityDraw,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntitySubmit) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            self.native_block_entity_submit(pos, block, state, &api, draw);
        }
    }
    pub(in crate::game) fn api_block_entity_render_bounding_box(
        &self,
        pos: [i32; 3],
        block: &str,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        bounds: &mut [f32; 6],
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityRenderBoundingBox) {
            return;
        }
        if let Ok(api) = entity.try_into_api() {
            self.native_block_entity_render_bounding_box(pos, block, state, &api, bounds);
        }
    }
    /// Shared click metadata borrows the already-converted API snapshot for this call.
    pub(in crate::game) fn block_click<'a>(
        &mut self,
        hit: &super::Hit,
        hand: mod_api::Hand,
        stack: Option<&'a ApiStack>,
    ) -> mod_api::BlockClick<'a> {
        let (player, player_entity_id) = self.native_clicker();
        mod_api::BlockClick::from(mod_api::__private::built::BlockClick {
            side: if self.remote.is_some() {
                mod_api::LogicalSide::Client
            } else {
                mod_api::LogicalSide::Server
            },
            dimension: self.dim.level(),
            pos: hit.pos,
            hit: hit.native(),
            hand,
            stack,
            player,
            player_entity_id,
        })
    }

    pub(in crate::game) fn api_menu_button(
        &mut self,
        context: &mod_api::item::menu::MenuButtonContext,
        entity: &mut crate::containers::ModBlockEntity,
        carried: &mut Option<EngineStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuButton) {
            return;
        }
        let (Ok(mut api), Ok(mut cursor)) = (entity.try_into_api(), carried.try_into_api()) else {
            return;
        };
        let revision = api.revision();
        let original = cursor.clone();
        self.native_menu_button(context, &mut api, &mut cursor);
        let cursor = if cursor == original {
            Ok(None)
        } else {
            cursor
                .try_into_engine(self.items())
                .map(Some)
                .map_err(|_| BlockEntitySnapshotError::InvalidStack)
        };
        if let Ok(Some(cursor)) = finish_block_entity_writeback(
            entity,
            &api,
            revision,
            self.items(),
            "menu_button",
            cursor,
        ) {
            *carried = cursor;
        }
    }
    pub(in crate::game) fn api_menu_quick_move(
        &mut self,
        pos: [i32; 3],
        entity: &mut crate::containers::ModBlockEntity,
        container_slots: usize,
        slot: usize,
        stack: &mut EngineStack,
        handled: &mut bool,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuQuickMove) {
            return;
        }
        let (Ok(mut api), Ok(mut held)) = (entity.try_into_api(), stack.try_into_api()) else {
            return;
        };
        let revision = api.revision();
        let original = held.clone();
        let mut candidate_handled = *handled;
        self.native_menu_quick_move(
            pos,
            &mut api,
            container_slots,
            slot,
            &mut held,
            &mut candidate_handled,
        );
        let held = if held == original {
            Ok(None)
        } else {
            held.try_into_engine(self.items())
                .map(Some)
                .map_err(|_| BlockEntitySnapshotError::InvalidStack)
        };
        if let Ok(held) = finish_block_entity_writeback(
            entity,
            &api,
            revision,
            self.items(),
            "menu_quick_move",
            held,
        ) {
            if let Some(held) = held {
                *stack = held;
            }
            *handled = candidate_handled;
        }
    }
}
impl super::GameLevel<'_> {
    pub(in crate::game) fn api_pre_remove_side_effects(
        &mut self,
        pos: horizon_core::position::BlockPos,
        state: horizon_world::registry::BlockStateId,
        entity: &crate::containers::ModBlockEntity,
        drops: &mut Vec<EngineStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::PreRemoveSideEffects) {
            return;
        }
        let Ok(api) = entity.try_into_api() else {
            return;
        };
        let mut added = Vec::new();
        self.native_pre_remove_side_effects(pos, state, &api, &mut added);
        drops.extend(added);
    }
}

#[cfg(test)]
#[path = "api_values/stacked_slot_tests.rs"]
mod stacked_slot_tests;

#[cfg(test)]
mod block_entity_snapshot_tests {
    use super::*;
    #[test]
    fn rejected_writebacks_invalidate_tick() {
        assert_rejected_writeback("block_entity_tick");
    }
    #[test]
    fn rejected_writebacks_invalidate_menu_button() {
        assert_rejected_writeback("menu_button");
    }
    #[test]
    fn rejected_writebacks_invalidate_menu_quick_move() {
        assert_rejected_writeback("menu_quick_move");
    }
    fn assert_rejected_writeback(hook: &'static str) {
        let items = Items::for_tests(Default::default());
        let mut entity =
            crate::containers::ModBlockEntity::new("test:rejected".into(), Default::default());
        let mut snapshot = entity.try_into_api().unwrap();
        let before = snapshot.revision();
        snapshot.data.push(42);
        let claimed = snapshot.revision().changes();
        // Tick rejection: wrong identity. Menu rejection: invalid cursor/held output.
        let output = if hook == "block_entity_tick" {
            snapshot = mod_api::ModBlockEntity::new("other:type".into(), Default::default());
            Ok(())
        } else {
            Err(BlockEntitySnapshotError::InvalidStack)
        };
        assert!(
            finish_block_entity_writeback(&mut entity, &snapshot, before, &items, hook, output)
                .is_err()
        );
        assert!(entity.data.is_empty());
        assert_eq!(
            entity.revision().changes(),
            if hook == "block_entity_tick" {
                1
            } else {
                claimed + 1
            }
        );
    }

    #[test]
    fn unchanged_snapshots_keep_revision_and_callback_writes_advance_it() {
        let mut entity =
            crate::containers::ModBlockEntity::new("test:machine".into(), Default::default());
        let first = entity.try_into_api().unwrap();
        let mut second = entity.try_into_api().unwrap();
        assert_eq!(first.revision(), second.revision());
        let before = second.revision();
        second.data.push(7);
        second.data.push(8);
        assert_ne!(before, second.revision());
        apply_block_entity_snapshot(
            &mut entity,
            &second,
            before,
            &Items::for_tests(Default::default()),
        )
        .unwrap();
        let third = entity.try_into_api().unwrap();
        assert_ne!(first.revision(), third.revision());
        assert_eq!(second.revision(), third.revision());
        assert_eq!(third.revision(), entity.try_into_api().unwrap().revision());
        assert_eq!(entity.id(), "test:machine");
        assert_eq!(entity.data, [7, 8]);
        let other = mod_api::ModBlockEntity::new("test:machine".into(), Default::default());
        assert_ne!(other.revision(), third.revision());
        entity.data.push(9);
        let fourth = entity.try_into_api().unwrap();
        assert_ne!(fourth.revision(), second.revision());
        let items = Items::for_tests(Default::default());
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &second, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &other, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert_eq!(entity.data, [7, 8, 9]);
    }

    #[test]
    fn a_callbacks_client_update_fields_reach_the_queue_only_from_a_valid_snapshot() {
        let items = Items::for_tests(Default::default());
        let mut entity =
            crate::containers::ModBlockEntity::new("test:machine".into(), Default::default());
        let batch =
            horizon_core::nbt::Compound::from([("effect".into(), horizon_core::nbt::Tag::Int(23))]);
        let made = entity.revision();
        let mut snapshot = entity.try_into_api().unwrap();
        let before = snapshot.revision();
        mod_api::ModBlockEntityView::new(&mut snapshot).set_client_update_fields(batch.clone());
        // Nothing but the queue moved, and an unchanged snapshot still delivers.
        apply_block_entity_snapshot(&mut entity, &snapshot, before, &items).unwrap();
        assert_eq!(entity.revision(), made);
        assert_eq!(entity.take_client_update_fields(), Some(batch.clone()));
        // A snapshot naming another entity is refused whole.
        let mut other = mod_api::ModBlockEntity::new("test:machine".into(), Default::default());
        let before = other.revision();
        mod_api::ModBlockEntityView::new(&mut other).set_client_update_fields(batch);
        assert_eq!(
            apply_block_entity_snapshot(&mut entity, &other, before, &items),
            Err(BlockEntitySnapshotError::InvalidRevision)
        );
        assert!(!entity.has_client_update_fields());
    }
}

#[cfg(test)]
mod mutable_output_tests {
    use super::*;

    #[test]
    fn array_outputs_are_admitted_only_after_every_stack_passes_resident_limits() {
        prepare_stack_boundary_tests();
        let root =
            std::env::temp_dir().join(format!("horizon-api-array-output-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("items.json"),
            r#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        let mut api = [
            ApiStack::new(key, 8).unwrap(),
            ApiStack::new(key, 65).unwrap(),
        ];
        let mut output = [
            EngineStack::new("minecraft:stick", 3),
            EngineStack::new("minecraft:stick", 4),
        ];
        let before = output.clone();
        let converted = api.try_into_engine(&items);
        assert_eq!(
            converted,
            Err(StackWriteError::CountExceeded { maximum: 64 })
        );
        if let Ok(validated) = converted {
            output = validated;
        }
        assert_eq!(
            output, before,
            "a legal first element must not be applied before a rejected second element"
        );

        api[1].set_count(64).unwrap();
        output = api.try_into_engine(&items).unwrap();
        assert_eq!(
            output,
            [
                EngineStack::new("minecraft:stick", 8),
                EngineStack::new("minecraft:stick", 64)
            ]
        );
    }
}

#[cfg(test)]
#[path = "api_values/read_stack_tests.rs"]
mod read_stack_tests;

#[cfg(test)]
mod stack_limit_tests {
    use super::*;

    pub(super) fn catalog() -> Items {
        prepare_stack_boundary_tests();
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-api-limits-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), r#"{
            "minecraft:stick":{"minecraft:max_stack_size":64},
            "minecraft:iron_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":10,"minecraft:damage":7},
            "minecraft:diamond_sword":{"minecraft:max_stack_size":1,"minecraft:max_damage":100,"minecraft:damage":0}
        }"#).unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items =
            Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        items
    }

    /// Vanilla stores damage above the maximum; only writes refuse it. Reads see the stack.
    #[test]
    fn a_write_refuses_damage_over_the_maximum_a_read_takes() {
        let items = catalog();
        let mut over = ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        over.set_damage(1000);
        assert_eq!(
            over.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 10 })
        );
        let read = stack_value(&over).unwrap();
        let definition = items.items.get("minecraft:iron_sword");
        assert_eq!(read.effective_damage(definition), 1000);
        assert!(horizon_client::stations::is_damageable(&items, &read));
    }

    #[test]
    fn strict_damage_limits_include_zero_inheritance_removal_and_nbt_int_range() {
        let mut items = catalog();
        let key = |id| mod_api::__private::item_key(id).unwrap();
        let plain = ApiStack::of(key("minecraft:stick"));
        assert!(plain.try_into_engine(&items).is_ok());
        let mut explicit = plain.clone();
        explicit.set_damage(0);
        let engine = explicit.try_into_engine(&items).unwrap();
        assert!(engine.carries_component(None, "minecraft:damage"));
        assert_eq!(
            horizon_items::item_data::encode_components(&engine)
                .unwrap()
                .get("minecraft:damage"),
            Some(&Tag::Int(0))
        );
        explicit.set_damage(u32::MAX);
        assert_eq!(
            explicit.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 0 })
        );

        let inherited = ApiStack::of(key("minecraft:iron_sword"));
        let definition = items.items.get("minecraft:iron_sword").unwrap();
        assert_eq!(
            inherited
                .try_into_engine(&items)
                .unwrap()
                .effective_damage(Some(definition)),
            7
        );
        let mut lower = inherited.clone();
        lower
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(5))].into())
            .unwrap();
        assert_eq!(
            lower.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 5 })
        );
        let mut removed = inherited;
        removed
            .set_components(
                [(
                    JavaString::from("!minecraft:max_damage"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        assert_eq!(
            removed.try_into_engine(&items),
            Err(StackWriteError::DamageExceeded { maximum: 0 })
        );
        removed.clear_damage();
        let engine = removed.try_into_engine(&items).unwrap();
        assert_eq!(engine.max_damage(Some(definition)), 0);
        assert_eq!(engine.effective_damage(Some(definition)), 0);
        removed.set_damage(0);
        assert!(removed.try_into_engine(&items).is_ok());

        items
            .items
            .get_mut("minecraft:diamond_sword")
            .unwrap()
            .max_damage = u32::MAX;
        let mut oversized = ApiStack::of(key("minecraft:diamond_sword"));
        oversized.set_damage(u32::MAX);
        assert_eq!(
            oversized.try_into_engine(&items),
            Err(StackWriteError::InvalidComponents {
                reason: mod_api::StackError::InvalidDamage
            })
        );
    }

    #[test]
    fn stack_maximum_respects_the_vanilla_codec_range() {
        let items = catalog();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        for maximum in [i32::MIN, -1, 0, 100, i32::MAX] {
            let mut stack = ApiStack::of(key);
            stack
                .set_components(
                    [(
                        JavaString::from("minecraft:max_stack_size"),
                        Tag::Int(maximum),
                    )]
                    .into(),
                )
                .unwrap();
            assert_eq!(
                stack.try_into_engine(&items),
                Err(StackWriteError::InvalidMaximum),
                "maximum {maximum}"
            );
        }
        for maximum in [1, 99] {
            let mut stack = ApiStack::new(key, maximum).unwrap();
            stack
                .set_components(
                    [(
                        JavaString::from("minecraft:max_stack_size"),
                        Tag::Int(maximum as i32),
                    )]
                    .into(),
                )
                .unwrap();
            assert_eq!(stack.try_into_engine(&items).unwrap().count, maximum);
        }
    }

    #[test]
    fn production_snapshots_preserve_component_corpus_contents() {
        let items = catalog();
        let cases = [
            ("minecraft:iron_sword", None),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:damage", Tag::Int(0))])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:damage", Tag::Int(5))])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([(
                    "!minecraft:damage",
                    Tag::Compound(Default::default()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:custom_name",
                    Tag::String("Named".into()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:custom_data",
                    Tag::compound([
                        ("list", Tag::List(vec![Tag::Int(3), Tag::Int(7)])),
                        ("bytes", Tag::ByteArray(vec![1, 2, 3])),
                    ]),
                )])),
            ),
            (
                "minecraft:iron_sword",
                Some(Tag::compound([("minecraft:max_stack_size", Tag::Int(1))])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([("minecraft:max_stack_size", Tag::Int(99))])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "!minecraft:max_stack_size",
                    Tag::Compound(Default::default()),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:container",
                    Tag::List(vec![Tag::compound([
                        ("slot", Tag::Int(0)),
                        (
                            "item",
                            Tag::compound([
                                ("id", Tag::String("minecraft:stick".into())),
                                ("count", Tag::Int(2)),
                            ]),
                        ),
                    ])]),
                )])),
            ),
            (
                "minecraft:stick",
                Some(Tag::compound([(
                    "minecraft:enchantments",
                    Tag::compound([("minecraft:sharpness", Tag::Int(2))]),
                )])),
            ),
        ];
        for (item, patch) in cases {
            let (damage, extra) = horizon_items::item_data::decode_components(patch.as_ref());
            let original = EngineStack {
                damage,
                extra,
                ..EngineStack::new(item, 1)
            };
            // The engine's canonical component codec is independent of the snapshot adapter.
            let expected = horizon_items::item_data::encode_extra_components(&original);
            let api = original.try_into_api().unwrap();
            assert_eq!(
                api.components()
                    .map(|components| Tag::Compound(components.as_nbt().clone())),
                expected,
                "read {patch:?}"
            );
            let back = api.try_into_engine(&items).unwrap();
            assert_eq!(
                horizon_items::item_data::encode_components(&original),
                horizon_items::item_data::encode_components(&back),
                "write {patch:?}"
            );
            assert_eq!(original, back, "round trip {patch:?}");
        }
    }

    #[test]
    fn durability_and_stackability_are_mutually_exclusive() {
        let items = catalog();
        let mut stick = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        stick
            .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(10))].into())
            .unwrap();
        assert_eq!(
            stick.try_into_engine(&items),
            Err(StackWriteError::StackableDamageable)
        );
        let mut sword = ApiStack::of(mod_api::__private::item_key("minecraft:iron_sword").unwrap());
        sword
            .set_components([(JavaString::from("minecraft:max_stack_size"), Tag::Int(2))].into())
            .unwrap();
        assert_eq!(
            sword.try_into_engine(&items),
            Err(StackWriteError::StackableDamageable)
        );
        stick
            .set_components(
                [
                    (JavaString::from("minecraft:max_damage"), Tag::Int(10)),
                    (JavaString::from("minecraft:max_stack_size"), Tag::Int(1)),
                ]
                .into(),
            )
            .unwrap();
        assert!(stick.try_into_engine(&items).is_ok());
        sword
            .set_components(
                [
                    (JavaString::from("minecraft:max_stack_size"), Tag::Int(2)),
                    (
                        JavaString::from("!minecraft:max_damage"),
                        Tag::Compound(Default::default()),
                    ),
                ]
                .into(),
            )
            .unwrap();
        sword.clear_damage();
        assert!(sword.try_into_engine(&items).is_ok());
    }

    #[test]
    fn removed_stack_maximum_uses_the_absent_component_limit_of_one() {
        let items = catalog();
        let mut stack =
            ApiStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 2).unwrap();
        stack
            .set_components(
                [(
                    JavaString::from("!minecraft:max_stack_size"),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        assert_eq!(
            stack.try_into_engine(&items),
            Err(StackWriteError::CountExceeded { maximum: 1 })
        );
        stack.set_count(1).unwrap();
        assert!(!stack.try_into_engine(&items).unwrap().carries_component(
            items.items.get("minecraft:stick"),
            "minecraft:max_stack_size"
        ));
    }

    #[test]
    fn damage_only_snapshots_preserve_presence_without_an_opaque_patch() {
        catalog();
        for damage in [0, 5] {
            let (damage, extra) = horizon_items::item_data::decode_components(Some(
                &Tag::compound([("minecraft:damage", Tag::Int(damage))]),
            ));
            let engine = EngineStack {
                damage,
                extra,
                ..EngineStack::new("minecraft:iron_sword", 1)
            };
            let api = engine.try_into_api().unwrap();
            assert_eq!(api.damage_component(), Some(damage));
            assert!(api.components().is_none());
        }
    }
}

#[cfg(test)]
mod item_model_snapshot_tests {
    use super::*;
    use horizon_assets::item_model::StackView;

    #[test]
    fn model_snapshots_keep_component_patches_without_runtime_bundle_selection() {
        prepare_stack_boundary_tests();
        let mut view = StackView::new("minecraft:stick", 2);
        view.bundle_selected = 3;
        view.patch
            .insert("minecraft:damage".into(), Some(serde_json::json!(7)));
        view.patch.insert(
            "example:charge".into(),
            Some(serde_json::json!({"amount":4})),
        );
        view.patch.insert("minecraft:custom_name".into(), None);
        let snapshot = view.try_into_api().unwrap();
        assert_eq!(
            snapshot.item(),
            mod_api::__private::item_key("minecraft:stick").unwrap()
        );
        assert_eq!(snapshot.count(), 2);
        assert_eq!(snapshot.damage(), 7);
        let patch = snapshot.components().unwrap().as_nbt();
        assert_eq!(
            patch
                .get(&JavaString::from("example:charge"))
                .unwrap()
                .get("amount"),
            Some(&Tag::Int(4))
        );
        assert!(
            patch
                .get(&JavaString::from("!minecraft:custom_name"))
                .is_some()
        );
        assert!(patch.get(&JavaString::from("bundle_selected")).is_none());
        assert_eq!(view.bundle_selected, 3);
    }

    #[test]
    fn empty_model_views_do_not_become_owned_stack_sentinels() {
        prepare_stack_boundary_tests();
        for count in [-1, 0] {
            assert_eq!(
                StackView::new("minecraft:stick", count).try_into_api(),
                Err(StackReadError::Empty)
            );
        }
        assert_eq!(
            StackView::new("example:unknown", 1).try_into_api(),
            Err(StackReadError::UnknownItem)
        );
    }
}

impl TryIntoApi for horizon_mod_package::Side {
    type Api = mod_api::Side;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match self {
            Self::Client => mod_api::Side::Client,
            Self::Both => mod_api::Side::Both,
        })
    }
}

fn package_snapshot(
    manifest: &horizon_mod_package::Manifest,
    kind: mod_api::PackageKind,
) -> mod_api::PackageInfo {
    mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        kind,
        manifest.side.try_into_api().unwrap(),
        std::iter::once(&manifest.id)
            .chain(&manifest.namespaces)
            .cloned()
            .collect(),
        manifest.dependencies.keys().cloned().collect(),
    )
}
impl TryIntoApi for horizon_mod_package::Manifest {
    type Api = mod_api::PackageInfo;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(package_snapshot(
            self,
            if self.wasm.is_some() {
                mod_api::PackageKind::Code
            } else {
                mod_api::PackageKind::Content
            },
        ))
    }
}
impl TryIntoApi for horizon_mod_package::packages::AdjacentManifest {
    type Api = mod_api::PackageInfo;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(package_snapshot(&self.manifest, mod_api::PackageKind::Code))
    }
}

pub(in crate::game) fn install_packages(
    native: &[horizon_mod_package::packages::AdjacentManifest],
    content: &[horizon_mod_package::selection::SelectedPackage],
) {
    let native: Vec<_> = native.iter().map(|p| p.try_into_api().unwrap()).collect();
    let content: Vec<_> = content
        .iter()
        .map(|p| p.package.manifest.try_into_api().unwrap())
        .collect();
    mod_api::__private::install_packages(&native, &content);
}

#[cfg(test)]
mod package_snapshot_tests {
    use super::*;
    #[test]
    fn package_metadata_keeps_declared_side_and_namespace_order() {
        for (side, expected) in [
            ("client", mod_api::Side::Client),
            ("both", mod_api::Side::Both),
        ] {
            let manifest = horizon_mod_package::Manifest::parse(&format!(
                "id='test'\nname='Test'\nversion='1'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\n"
            )).unwrap();
            let snapshot = manifest.try_into_api().unwrap();
            assert_eq!(snapshot.side(), expected);
            assert_eq!(snapshot.namespaces().collect::<Vec<_>>(), ["test"]);
            assert_eq!(snapshot.kind(), mod_api::PackageKind::Content);
        }
    }
}

impl TryIntoApi for horizon_client_core::suggestion::Suggestion {
    type Api = mod_api::Suggestion;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let mut value = mod_api::Suggestion::new(self.range, self.text.clone());
        value.tooltip.clone_from(&self.tooltip);
        value.int_value = self.int_value;
        Ok(value)
    }
}
impl TryIntoEngine for mod_api::Suggestion {
    type Engine = horizon_client_core::suggestion::Suggestion;
    type Error = StackWriteError;
    fn try_into_engine(&self, _: &Items) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            range: self.range,
            text: self.text.clone(),
            tooltip: self.tooltip.clone(),
            int_value: self.int_value,
        })
    }
}
#[cfg(test)]
mod completion_snapshot_tests {
    use super::*;
    #[test]
    fn completion_round_trip_keeps_range_tooltip_and_numeric_order() {
        let source = horizon_client_core::suggestion::Suggestion {
            range: (4, 7),
            text: "42".into(),
            tooltip: Some("count".into()),
            int_value: Some(42),
        };
        let mut api = source.try_into_api().unwrap();
        api.text = "43".into();
        api.int_value = Some(43);
        let engine = api
            .try_into_engine(&Items::for_tests(Default::default()))
            .unwrap();
        assert_eq!(engine.range, (4, 7));
        assert_eq!(engine.tooltip.as_deref(), Some("count"));
        assert_eq!(engine.int_value, Some(43));
        assert_eq!(engine.apply("foo 123 bar"), "foo 43 bar");
        assert_eq!(source.text, "42");
    }
}

// Registration declarations do not need a live item registry.
impl TryIntoEngine<()> for mod_api::BlockEntityTypeDecl {
    type Engine = horizon_mod_package::content::BlockEntityType;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            id: self.id.clone(),
            blocks: self.blocks.clone(),
        })
    }
}
pub(in crate::game) fn block_entity_declarations()
-> Vec<horizon_mod_package::content::BlockEntityType> {
    mod_api::__private::block_entity_types()
        .iter()
        .map(|decl| decl.try_into_engine(&()).unwrap())
        .collect()
}

impl TryIntoEngine<()> for mod_api::GameRuleDecl {
    type Engine = horizon_client_core::gamerules::Rule;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            name: self.name.clone(),
            category: self.category.clone(),
            default: horizon_client_core::gamerules::Value::Bool(self.default),
            min: 0,
            max: 0,
        })
    }
}

#[cfg(test)]
mod declaration_snapshot_tests {
    use super::*;
    #[test]
    fn declarations_keep_supported_blocks_and_rule_positions() {
        static KEY: std::sync::OnceLock<mod_api::BoolGameRuleKey> = std::sync::OnceLock::new();
        // Use registration, the owner of declaration validation.
        struct Mod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Mod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.declare_block_entity_type(
                    "machine",
                    vec!["test:second".into(), "test:first".into()],
                );
                r.game_rule_category("machines").unwrap();
                KEY.set(r.game_rule_bool("active", "test:machines", true).unwrap())
                    .unwrap();
            }
        }
        install_parts(&[mod_api::ModEntry::new("test", Box::new(Mod))]).unwrap();
        let declarations = block_entity_declarations();
        assert_eq!(declarations[0].id, "test:machine");
        assert_eq!(declarations[0].blocks, ["test:second", "test:first"]);
        let registry = game_rule_registry();
        let key = *KEY.get().unwrap();
        assert_eq!(
            mod_api::__private::game_rule_bool_slot(key),
            registry.position("test:active")
        );
        assert_eq!(
            registry.rule("test:active").unwrap().default,
            horizon_client_core::gamerules::Value::Bool(true)
        );
    }
}

macro_rules! gui_draw_value {
    ($draw:expr, $from:ident, $to:ident, $convert:ident($($args:expr),*)) => {
        match $draw {
            $from::Sprite {
                sprite,
                x,
                y,
                w,
                h,
                region,
                color,
                blend,
            } => $to::Sprite {
                sprite: sprite.clone(),
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                region: *region,
                color: *color,
                blend: *blend,
            },
            $from::AtlasSprite {
                sprite,
                x,
                y,
                w,
                h,
                color,
            } => $to::AtlasSprite {
                sprite: sprite.clone(),
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                color: *color,
            },
            $from::Blit {
                texture,
                blend,
                x,
                y,
                w,
                h,
                uv,
                src,
                tex,
                color,
            } => $to::Blit {
                texture: texture.clone(),
                blend: *blend,
                x: *x,
                y: *y,
                w: *w,
                h: *h,
                uv: *uv,
                src: *src,
                tex: *tex,
                color: *color,
            },
            $from::Fill {
                x0,
                y0,
                x1,
                y1,
                color,
            } => $to::Fill {
                x0: *x0,
                y0: *y0,
                x1: *x1,
                y1: *y1,
                color: *color,
            },
            $from::Text {
                text,
                x,
                y,
                color,
                shadow,
            } => $to::Text {
                text: text.$convert($($args),*)?,
                x: *x,
                y: *y,
                color: *color,
                shadow: *shadow,
            },
            $from::Item {
                slot,
                x,
                y,
                seed,
                scale,
                pivot,
            } => $to::Item {
                slot: *slot,
                x: *x,
                y: *y,
                seed: *seed,
                scale: *scale,
                pivot: *pivot,
            },
            $from::Push(ops) => $to::Push(ops.clone()),
            $from::Pop => $to::Pop,
        }
    };
}

impl TryIntoApi for horizon_client::hud_layout::GuiDraw {
    type Api = mod_api::client::hud_layout::GuiDraw;
    type Error = mod_api::text::TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        use horizon_client::hud_layout::GuiDraw as Engine;
        use mod_api::client::hud_layout::GuiDraw as Api;
        Ok(gui_draw_value!(self, Engine, Api, try_into_api()))
    }
}
impl<C: ?Sized> TryIntoEngine<C> for mod_api::client::hud_layout::GuiDraw {
    type Engine = horizon_client::hud_layout::GuiDraw;
    type Error = mod_api::text::TextFormError;
    fn try_into_engine(&self, _: &C) -> Result<Self::Engine, Self::Error> {
        use horizon_client::hud_layout::GuiDraw as Engine;
        use mod_api::client::hud_layout::GuiDraw as Api;
        Ok(gui_draw_value!(self, Api, Engine, try_into_engine(&())))
    }
}

#[path = "api_values/text_boundary.rs"]
mod text_boundary;
pub(in crate::game) use text_boundary::style_hover;
#[cfg(test)]
mod gui_draw_boundary_tests {
    use super::{TryIntoApi, TryIntoEngine};
    use horizon_client::{
        hud_layout::{Blend, GuiDraw as D, PoseOp},
        text::Text,
    };
    use serde_json::json;

    #[test]
    fn gui_commands_keep_every_variant_field_order_and_text_source() {
        let text = Text::from(
            json!({"translate":"test.hud", "fallback":"%s", "with":[{"text":"name"}], "bold":false, "test:meta":{"n":1}}),
        );
        let commands = vec![
            D::Sprite {
                sprite: String::from("test:sprite").into(),
                x: -1,
                y: 2,
                w: 3,
                h: 4,
                region: Some([5, 6, 7, 8]),
                color: 0x12345678,
                blend: Blend::Crosshair,
            },
            D::AtlasSprite {
                sprite: "test:atlas".into(),
                x: 11,
                y: 12,
                w: 13,
                h: 14,
                color: 15,
            },
            D::Blit {
                texture: "test:texture".into(),
                blend: Blend::Nausea,
                x: 21,
                y: 22,
                w: 23,
                h: 24,
                uv: [0.25, 0.5],
                src: [26, 27],
                tex: [28, 29],
                color: 30,
            },
            D::Fill {
                x0: -31,
                y0: 32,
                x1: 33,
                y1: 34,
                color: 35,
            },
            D::Text {
                text: text.clone(),
                x: 41,
                y: 42,
                color: 43,
                shadow: false,
            },
            D::Item {
                slot: 40,
                x: 51,
                y: 52,
                seed: -53,
                scale: [0.5, 1.5],
                pivot: [55., 56.],
            },
            D::Push(vec![PoseOp::Translate(61., 62.), PoseOp::Scale(0.5, 1.5)]),
            D::Pop,
        ];
        let api = commands.try_into_api().unwrap();
        let actual: Result<Vec<D>, mod_api::text::TextFormError> = api.try_into_engine(&());
        let actual = actual.unwrap();
        assert_eq!(actual, commands);
        let D::Text { text: actual, .. } = &actual[4] else {
            panic!("text command")
        };
        assert_eq!(actual.component(), text.component());
    }

    #[test]
    fn tooltip_pairs_round_trip_without_an_item_registry() {
        let lines = vec![(horizon_client::text::Text::literal("tip"), 0xff00ff00)];
        let snapshot: Result<Vec<(mod_api::Text, u32)>, mod_api::text::TextFormError> =
            lines.try_into_api();
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot[0], (mod_api::Text::literal("tip"), 0xff00ff00));
        let restored: Result<Vec<(horizon_client::text::Text, u32)>, mod_api::text::TextFormError> =
            snapshot.try_into_engine(&());
        let restored = restored.unwrap();
        assert_eq!(restored[0].0.component(), lines[0].0.component());
        assert_eq!(restored[0].0.spans, lines[0].0.spans);
        assert_eq!(restored[0].1, lines[0].1);
    }

    #[test]
    fn nested_text_containers_need_no_item_registry_or_stack_error() {
        let pair = (
            mod_api::text::Text::literal("a"),
            mod_api::text::Text::literal("b"),
        );
        let values = Some(vec![[pair.clone(), pair]]);
        let reference = &values;
        let actual: Result<_, mod_api::text::TextFormError> = reference.try_into_engine(&());
        let actual = actual.unwrap().unwrap();
        assert_eq!(actual[0][1].0.plain(), "a");
        assert_eq!(actual[0][1].1.plain(), "b");
    }
}

impl TryIntoApi for horizon_world::level::PlacementContext {
    type Api = mod_api::PlacementContext;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let item = if self.item.is_empty() {
            None
        } else {
            Some(mod_api::__private::item_key(&self.item).ok_or(StackReadError::UnknownItem)?)
        };
        let mut snapshot = mod_api::PlacementContext::new(self.pos, self.face, item);
        snapshot.hit_pos = self.hit_pos;
        snapshot.hit = [self.hit.x, self.hit.y, self.hit.z];
        snapshot.inside = self.inside;
        snapshot.yaw = self.yaw;
        snapshot.pitch = self.pitch;
        snapshot.sneaking = self.sneaking;
        snapshot.replace_clicked = self.replace_clicked;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod placement_snapshot_tests {
    use super::*;
    #[test]
    fn placement_keeps_click_view_flags_and_empty_hand_without_fallback() {
        install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stone"]).unwrap();
        let mut engine = horizon_world::level::PlacementContext {
            pos: horizon_core::position::BlockPos::new(1, 2, 3),
            hit_pos: horizon_core::position::BlockPos::new(4, 5, 6),
            face: horizon_core::geometry::Direction::Up,
            hit: horizon_core::geometry::Vec3::new(4.25, 5.5, 6.75),
            inside: true,
            yaw: -45.0,
            pitch: 90.0,
            sneaking: true,
            item: "minecraft:stone".into(),
            replace_clicked: true,
        };
        let snapshot = engine.try_into_api().unwrap();
        assert_eq!(snapshot.pos, horizon_core::position::BlockPos::new(1, 2, 3));
        assert_eq!(
            snapshot.hit_pos,
            horizon_core::position::BlockPos::new(4, 5, 6)
        );
        assert_eq!(snapshot.face, horizon_core::geometry::Direction::Up);
        assert_eq!(snapshot.hit, [4.25, 5.5, 6.75]);
        assert!(snapshot.inside);
        assert_eq!(snapshot.yaw, -45.0);
        assert_eq!(snapshot.pitch, 90.0);
        assert!(snapshot.sneaking && snapshot.replace_clicked);
        assert_eq!(
            snapshot.item,
            mod_api::__private::item_key("minecraft:stone")
        );
        engine.item.clear();
        let mut empty = snapshot;
        empty.item = None;
        assert_eq!(engine.try_into_api(), Ok(empty));
        engine.item = "missing:item".into();
        assert_eq!(engine.try_into_api(), Err(StackReadError::UnknownItem));
    }
}

impl TryIntoApi for horizon_client_core::gamerules::Value {
    type Api = mod_api::GameRuleValue;
    type Error = StackReadError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        Ok(match *self {
            Self::Bool(value) => mod_api::GameRuleValue::Bool(value),
            Self::Int(value) => mod_api::GameRuleValue::Int(value),
        })
    }
}

#[test]
fn game_rule_int_snapshot_preserves_full_range() {
    for value in [i32::MIN, -1, 0, 256, i32::MAX] {
        assert_eq!(
            horizon_client_core::gamerules::Value::Int(value)
                .try_into_api()
                .unwrap(),
            mod_api::GameRuleValue::Int(value)
        );
    }
}

fn with_rule_schema<T>(f: impl FnOnce(&mod_api::__private::RuleSchema<'_>) -> T) -> T {
    let registry = horizon_client_core::gamerules::rule_registry();
    f(&mod_api::__private::RuleSchema {
        category: &|id| registry.categories().iter().any(|category| category == id),
        position: &|id| registry.position(id),
        rule_count: u32::try_from(registry.available_rules(true).count())
            .expect("vanilla rule count fits u32"),
    })
}
pub(in crate::game) fn install_parts(mods: &[mod_api::ModEntry]) -> Result<(), Vec<String>> {
    with_rule_schema(|schema| {
        mod_api::__private::install_parts(mods, schema, super::native_dimensions::valid_shape)
    })
}
#[cfg(test)]
pub(in crate::game) fn register_game_keys(ids: impl IntoIterator<Item = String>) {
    with_rule_schema(|schema| {
        mod_api::__private::part_tests::register_game_keys(
            ids,
            schema,
            super::native_dimensions::valid_shape,
        )
    });
    mod_api::__private::resolve_effects(
        horizon_client_core::potions::all_effects()
            .map(|def| horizon_client_core::potions::namespaced(def.id)),
        |id| horizon_client_core::potions::effect(id).map(|def| def.index),
    )
    .unwrap();
    if let Some(types) = horizon_client_core::block_entity_types::current() {
        mod_api::__private::resolve_block_entity_types(types.names(), |id| types.key(id)).unwrap();
    }
}

pub(in crate::game) fn game_rule_registry() -> horizon_client_core::gamerules::RuleRegistry {
    let rules: Vec<_> = mod_api::__private::game_rules()
        .iter()
        .map(|rule| rule.try_into_engine(&()).unwrap())
        .collect();
    horizon_client_core::gamerules::RuleRegistry::with_bool_rules(
        mod_api::__private::game_rule_categories(),
        &rules,
    )
    .expect("validated at registration")
}

#[path = "api_values/damage.rs"]
pub(in crate::game) mod damage;
#[path = "api_values/item_properties.rs"]
mod item_properties;
#[path = "api_values/loot_values.rs"]
mod loot_values;
#[path = "api_values/particle_values.rs"]
mod particle_values;
pub(crate) use loot_values::{dispatch_loot_condition, dispatch_loot_function};
#[path = "api_values/phase_values.rs"]
mod phase_values;
#[path = "api_values/small_values.rs"]
mod small_values;
