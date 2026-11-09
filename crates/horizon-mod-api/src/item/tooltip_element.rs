use crate::{ItemStack, Text};

/// One element of an item's tooltip as `render_tooltip_gather_components`
/// hands it over, before any wrapping
/// (`Either<FormattedText, TooltipComponent>`). Text deeper than 512 nodes
/// refuses tooltip conversion before any changed element is drawn.
#[derive(Clone, PartialEq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum TooltipElement {
    /// A line (`FormattedText`) with its colour as `rgba` packs it
    /// (`u32::MAX` keeps the text's own).
    Text(Text, u32),
    /// An image component (`TooltipComponent`).
    Image(TooltipImage),
}

/// An image component of a tooltip: `size` GUI pixels, drawn by the
/// `tooltip_component_draw` part of `stack`'s item, with `stack`'s data.
/// An item's own `tooltip_component` part answers the one it starts with.
#[derive(Clone, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct TooltipImage {
    /// The stack.
    pub stack: ItemStack,
    /// The size.
    pub size: [f32; 2],
}

/// Where a tooltip hangs from, in GUI pixels: what `render_tooltip_pre` lets a mod move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods change both fields
#[derive(horizon_mod_api_macros::Wire)]
pub struct TooltipPlace {
    /// The point's x, before the positioner's offset and clamp; set it to move the tooltip.
    pub x: i32,
    /// The point's y, before the positioner's offset and clamp; set it to move the tooltip.
    pub y: i32,
}
