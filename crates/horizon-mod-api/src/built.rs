#[allow(unused_imports)]
use super::*;
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ToolUse {
    pub player: Option<u32>,
    pub hand: crate::client::first_person::Hand,
    pub pos: [i32; 3],
    pub face: [i32; 3],
    pub click: [f64; 3],
    pub ability: ToolAbility,
    pub simulated: bool,
}
impl From<ToolUse> for super::ToolUse {
    fn from(v: ToolUse) -> Self {
        let ToolUse {
            player,
            hand,
            pos,
            face,
            click,
            ability,
            simulated,
        } = v;
        Self {
            player,
            hand,
            pos,
            face,
            click,
            ability,
            simulated,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrowed game-to-hook adapter; its call fields cross separately.
pub struct LootDrops<'a> {
    pub table: &'a str,
    pub dimension: &'a str,
    pub origin: [f64; 3],
    pub block: Option<([i32; 3], BlockState)>,
    pub entity: Option<(u32, &'a str)>,
    pub player: Option<u128>,
    pub tool: Option<&'a ItemStack>,
    pub explosion: Option<f32>,
    pub used: bool,
}
impl<'a> From<LootDrops<'a>> for super::LootDrops<'a> {
    fn from(v: LootDrops<'a>) -> Self {
        let LootDrops {
            table,
            dimension,
            origin,
            block,
            entity,
            player,
            tool,
            explosion,
            used,
        } = v;
        Self {
            table,
            dimension,
            origin,
            block,
            entity,
            player,
            tool,
            explosion,
            used,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrowed game-to-hook adapter; its call fields cross separately.
pub struct ExplosionStart<'a> {
    pub dimension: &'a str,
    pub center: [f64; 3],
    pub radius: f32,
    pub fire: bool,
    pub interaction: BlockInteraction,
    pub damage_type: &'a str,
    pub source: Option<u32>,
    pub owner: Option<u32>,
}
impl<'a> From<ExplosionStart<'a>> for super::ExplosionStart<'a> {
    fn from(v: ExplosionStart<'a>) -> Self {
        let ExplosionStart {
            dimension,
            center,
            radius,
            fire,
            interaction,
            damage_type,
            source,
            owner,
        } = v;
        Self {
            dimension,
            center,
            radius,
            fire,
            interaction,
            damage_type,
            source,
            owner,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrowed game-to-hook adapter; its call fields cross separately.
pub struct ItemFished<'a> {
    pub hook: u32,
    pub position: [f64; 3],
    pub drops: &'a [ItemStack],
}
impl<'a> From<ItemFished<'a>> for super::ItemFished<'a> {
    fn from(v: ItemFished<'a>) -> Self {
        let ItemFished {
            hook,
            position,
            drops,
        } = v;
        Self {
            hook,
            position,
            drops,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrowed game-to-hook adapter; its call fields cross separately.
pub struct ScreenView<'a> {
    pub id: &'static str,
    pub menu: Option<&'a str>,
    pub title: Option<&'a Text>,
    pub rect: [f32; 4],
    pub slots: usize,
    pub mouse: [f32; 2],
    pub partial_tick: f32,
}
impl<'a> From<ScreenView<'a>> for super::ScreenView<'a> {
    fn from(v: ScreenView<'a>) -> Self {
        let ScreenView {
            id,
            menu,
            title,
            rect,
            slots,
            mouse,
            partial_tick,
        } = v;
        Self {
            id,
            menu,
            title,
            rect,
            slots,
            mouse,
            partial_tick,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrowed game-to-hook adapter; its call fields cross separately.
pub struct ItemClick<'a> {
    pub side: LogicalSide,
    pub dimension: LevelDimension,
    pub pos: [i32; 3],
    pub hand: Hand,
    pub stack: &'a ItemStack,
    pub player: u128,
    pub player_entity_id: i32,
}
impl<'a> From<ItemClick<'a>> for super::ItemClick<'a> {
    fn from(v: ItemClick<'a>) -> Self {
        let ItemClick {
            side,
            dimension,
            pos,
            hand,
            stack,
            player,
            player_entity_id,
        } = v;
        Self {
            side,
            dimension,
            pos,
            hand,
            stack,
            player,
            player_entity_id,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrowed game-to-hook adapter; its call fields cross separately.
pub struct BlockClick<'a> {
    pub side: LogicalSide,
    pub dimension: LevelDimension,
    pub pos: [i32; 3],
    pub hit: super::BlockHit,
    pub hand: Hand,
    pub stack: Option<&'a ItemStack>,
    pub player: u128,
    pub player_entity_id: i32,
}
impl<'a> From<BlockClick<'a>> for super::BlockClick<'a> {
    fn from(v: BlockClick<'a>) -> Self {
        let BlockClick {
            side,
            dimension,
            pos,
            hit,
            hand,
            stack,
            player,
            player_entity_id,
        } = v;
        Self {
            side,
            dimension,
            pos,
            hit,
            hand,
            stack,
            player,
            player_entity_id,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct LifecycleContext {
    pub authority: Authority,
    pub epoch: super::WorldEpoch,
    pub tick: super::TickId,
}
impl From<LifecycleContext> for super::LifecycleContext {
    fn from(v: LifecycleContext) -> Self {
        let LifecycleContext {
            authority,
            epoch,
            tick,
        } = v;
        Self {
            authority,
            epoch,
            tick,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ActiveEffect {
    pub id: super::EffectKey,
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    pub visible: bool,
    pub show_icon: bool,
}
impl From<ActiveEffect> for super::ActiveEffect {
    fn from(v: ActiveEffect) -> Self {
        let ActiveEffect {
            id,
            duration,
            amplifier,
            ambient,
            visible,
            show_icon,
        } = v;
        Self {
            id,
            duration,
            amplifier,
            ambient,
            visible,
            show_icon,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerExperience {
    pub level: i32,
    pub progress: f32,
    pub total: i32,
}
impl From<PlayerExperience> for super::PlayerExperience {
    fn from(v: PlayerExperience) -> Self {
        let PlayerExperience {
            level,
            progress,
            total,
        } = v;
        Self {
            level,
            progress,
            total,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerFood {
    pub level: i32,
    pub saturation: f32,
    pub exhaustion: f32,
}
impl From<PlayerFood> for super::PlayerFood {
    fn from(v: PlayerFood) -> Self {
        let PlayerFood {
            level,
            saturation,
            exhaustion,
        } = v;
        Self {
            level,
            saturation,
            exhaustion,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct PlayerAbilities {
    pub invulnerable: bool,
    pub flying: bool,
    pub may_fly: bool,
    pub instabuild: bool,
    pub may_build: bool,
    pub fly_speed: f32,
    pub walk_speed: f32,
}
impl From<PlayerAbilities> for super::PlayerAbilities {
    fn from(v: PlayerAbilities) -> Self {
        let PlayerAbilities {
            invulnerable,
            flying,
            may_fly,
            instabuild,
            may_build,
            fly_speed,
            walk_speed,
        } = v;
        Self {
            invulnerable,
            flying,
            may_fly,
            instabuild,
            may_build,
            fly_speed,
            walk_speed,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrowed game-to-hook adapter; its call fields cross separately.
pub struct ServerPlayer {
    pub uuid: u128,
    pub name: String,
    pub permission: u8,
    pub dimension: String,
    pub position: [f64; 3],
    pub game_mode: i32,
    pub health: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub entity_id: i32,
    pub address: String,
}
impl From<ServerPlayer> for super::ServerPlayer {
    fn from(v: ServerPlayer) -> Self {
        let ServerPlayer {
            uuid,
            name,
            permission,
            dimension,
            position,
            game_mode,
            health,
            yaw,
            pitch,
            entity_id,
            address,
        } = v;
        Self {
            uuid,
            name,
            permission,
            dimension,
            position,
            game_mode,
            health,
            yaw,
            pitch,
            entity_id,
            address,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ChannelRef {
    pub id: u64,
}
impl From<ChannelRef> for super::ChannelRef {
    fn from(v: ChannelRef) -> Self {
        let ChannelRef { id } = v;
        Self { id }
    }
}
// exhaustive: the game builds it with a literal; mods read it and edit its fields
#[derive(horizon_mod_api_macros::Wire)]
pub struct SoundPlay {
    pub identifier: String,
    pub source: super::SoundSource,
    pub volume: f32,
    pub pitch: f32,
    pub position: [f64; 3],
    pub looping: bool,
    pub delay: i32,
    pub attenuation: super::SoundAttenuation,
    pub relative: bool,
}
impl From<SoundPlay> for super::SoundPlay {
    fn from(v: SoundPlay) -> Self {
        let SoundPlay {
            identifier,
            source,
            volume,
            pitch,
            position,
            looping,
            delay,
            attenuation,
            relative,
        } = v;
        Self {
            identifier,
            source,
            volume,
            pitch,
            position,
            looping,
            delay,
            attenuation,
            relative,
        }
    }
}
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct PortalShape {
    pub x_axis: bool,
    pub bottom_left: [i32; 3],
    pub width: i32,
    pub height: i32,
}
impl From<PortalShape> for super::PortalShape {
    fn from(v: PortalShape) -> Self {
        let PortalShape {
            x_axis,
            bottom_left,
            width,
            height,
        } = v;
        Self {
            x_axis,
            bottom_left,
            width,
            height,
        }
    }
}
