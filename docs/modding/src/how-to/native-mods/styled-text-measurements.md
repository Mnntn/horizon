<!-- chapter: Measuring styled text outside drawing -->
<!-- separator: 1 -->

### Measuring styled text outside drawing

A tooltip component can determine its dimensions before it receives any draw
context. Use `ClientHost::text_width(&Text)` and `text_split(&Text, width)`:
these are reads of the client's active font in unscaled GUI pixels, also usable
in client setup. Native and wasm mods use the same methods.

Construct an owned `Text` with its content, style and children. A bridge with a
flat component representation can build a `TextForm` and call
`Text::try_from(form)` once at its boundary; the host reads accept that validated
`Text`, not JSON or engine text types. Both methods return a `TextFormError` if
the supplied component cannot be represented at the engine boundary.

Width is the same styled and translated width used by the engine's tooltip
layout. Wrapping follows `Font.split(FormattedText, width)`: it keeps styles,
breaks at newlines and fitting spaces, and uses the vanilla one-pixel minimum
for widths below one. Returned lines are API-owned `Text` values. Translation
uses the current client language; a later language or resource-pack font change
is reflected by the next call, so do not retain dimensions across reloads.

NeoForge's `ClientTooltipComponent` receives a `Font` in its width and drawing
methods. These reads use that same role—the active client font—without passing
an engine font object or requiring a `DrawContext`. They do not draw anything
and do not expose a full rendering context.
