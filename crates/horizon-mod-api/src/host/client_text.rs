//! Styled text measurements using the client's active font, outside drawing.
#![allow(unused_variables)]
use crate::*;

/// Read-only access to the font used by client tooltip layout.
#[horizon_mod_api_macros::host_area]
pub trait ClientTextClientHost {
    /// Side: client.
    /// `Font.width(FormattedText)`: the width of a component in unscaled GUI
    /// pixels, using the current language, styles and resource-pack fonts.
    /// Available outside a draw, including client setup and tooltip-component
    /// sizing. Like `ClientTooltipComponent.getWidth(Font)`, this measures with
    /// the rendering client's active font, not a font chosen by the mod.
    /// Build owned components with `Text`, or validate a flat `TextForm` with
    /// `Text::try_from(form)`. Invalid component input returns its boundary error.
    #[access(ClientSetup)]
    fn text_width(&self, text: &Text) -> Result<i32, TextFormError>;

    /// Side: client.
    /// `Font.split(FormattedText, width)`: wrap a component at `width` unscaled
    /// GUI pixels. Lines retain styles, resolve the current language and use
    /// the same active font as tooltip layout. Newlines force breaks; a width
    /// below one follows vanilla's one-pixel minimum, without rejecting input.
    /// Available outside drawing on native and wasm alike.
    #[access(ClientSetup)]
    fn text_split(&self, text: &Text, width: i32) -> Result<Vec<Text>, TextFormError>;
}
