//! Owned drawing data for a screen scene, separate from live world entities.

use crate::{BlockState, Identifier, item::containers::ModBlockEntityFields};

/// A mod block entity's type and public fields supplied to a screen's drawing Part.
/// This is not a live entity: internal cache and revision identities never cross the API.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneBlockEntity {
    /// The immutable type id of the temporary drawing entity.
    pub id: Identifier,
    /// Saved fields, item slots and menu data, in their existing public form.
    pub fields: ModBlockEntityFields,
}

impl SceneBlockEntity {
    /// Describe an entity to draw without retaining a world entity or its cache identity.
    pub fn new(id: Identifier, fields: ModBlockEntityFields) -> Self {
        Self { id, fields }
    }
}

/// One block of a scene a mod draws in a screen: its local position and state, and the
/// optional mod block entity drawn by its Part as in the world.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneBlock {
    /// Position in the scene, independent of the world's blocks.
    pub pos: [i32; 3],
    /// Owned block state, including properties, whose baked model is drawn.
    /// An invalid or unavailable state refuses the entire scene before drawing.
    pub state: BlockState,
    /// Optional owned block entity drawing data.
    pub entity: Option<SceneBlockEntity>,
}

impl SceneBlock {
    /// Describe one scene-local block and optional entity.
    pub fn new(pos: [i32; 3], state: BlockState, entity: Option<SceneBlockEntity>) -> Self {
        Self { pos, state, entity }
    }
}

/// How a scene is seen: its center, vertical rotation and top tilt in degrees, and
/// the number of GUI pixels a block spans.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct SceneView {
    /// Scene point placed at the GUI rectangle's center, in blocks.
    pub center: [f32; 3],
    /// Counterclockwise rotation seen from above, in degrees.
    pub yaw: f32,
    /// Tilt of the scene's top toward the viewer, in degrees.
    pub pitch: f32,
    /// GUI pixels per block; a nonpositive or nonfinite scale draws nothing.
    pub scale: f32,
}

impl SceneView {
    /// Set the scene's camera using the same units as its public fields.
    pub fn new(center: [f32; 3], yaw: f32, pitch: f32, scale: f32) -> Self {
        Self {
            center,
            yaw,
            pitch,
            scale,
        }
    }
}
