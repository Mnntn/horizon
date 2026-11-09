//! The parts of a humanoid model as vanilla posed them this frame (`HumanoidModel.setupAnim`),
//! read-only, for a mod drawing on a player or humanoid mob in `mob_layer`. The poses are the
//! [`PartPose`](crate::PartPose)s a baked layer's parts take, so a mod copies them onto its own
//! tree (`ClientHost::bake_layer`) and draws it where vanilla drew the body.

/// A part of a humanoid model, by the name vanilla's `PlayerModel` gives it. The outer skin
/// layer's parts (`Hat`, `Jacket`, the sleeves and the pants) are children of the part they cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum HumanoidPart {
    /// The head.
    Head,
    /// The hat layer, over the head.
    Hat,
    /// The body.
    Body,
    /// The jacket layer, over the body.
    Jacket,
    /// The right arm.
    RightArm,
    /// The right sleeve, over the right arm.
    RightSleeve,
    /// The left arm.
    LeftArm,
    /// The left sleeve, over the left arm.
    LeftSleeve,
    /// The right leg.
    RightLeg,
    /// The right trouser leg, over the right leg.
    RightPants,
    /// The left leg.
    LeftLeg,
    /// The left trouser leg, over the left leg.
    LeftPants,
}

impl HumanoidPart {
    /// Every part, parents before their children.
    pub const ALL: [Self; 12] = [
        Self::Head,
        Self::Hat,
        Self::Body,
        Self::Jacket,
        Self::RightArm,
        Self::RightSleeve,
        Self::LeftArm,
        Self::LeftSleeve,
        Self::RightLeg,
        Self::RightPants,
        Self::LeftLeg,
        Self::LeftPants,
    ];

    /// Its name in the model (`right_sleeve`).
    pub fn model_name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Hat => "hat",
            Self::Body => "body",
            Self::Jacket => "jacket",
            Self::RightArm => "right_arm",
            Self::RightSleeve => "right_sleeve",
            Self::LeftArm => "left_arm",
            Self::LeftSleeve => "left_sleeve",
            Self::RightLeg => "right_leg",
            Self::RightPants => "right_pants",
            Self::LeftLeg => "left_leg",
            Self::LeftPants => "left_pants",
        }
    }
}

/// The humanoid parts a mod hid from the vanilla player model this frame (`player_submit`): a
/// set over [`HumanoidPart::ALL`], nothing allocated. A hidden part takes its children with it
/// (the hat goes with the head); the cape is a layer of its own and stays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct HiddenParts {
    /// One bit per part, at its index in [`HumanoidPart::ALL`].
    pub bits: u16,
}

impl HiddenParts {
    /// Leaves `part` out of the vanilla model this frame.
    pub fn hide(&mut self, part: HumanoidPart) {
        self.bits |= 1 << part as u16;
    }

    /// Whether `part` is hidden.
    pub fn contains(self, part: HumanoidPart) -> bool {
        self.bits & (1 << part as u16) != 0
    }

    /// No part is hidden.
    pub fn is_empty(self) -> bool {
        self.bits == 0
    }

    /// The hidden parts, in [`HumanoidPart::ALL`]'s order.
    pub fn iter(self) -> impl Iterator<Item = HumanoidPart> {
        HumanoidPart::ALL
            .into_iter()
            .filter(move |part| self.contains(*part))
    }
}

/// One humanoid part as posed this frame: its pose relative to its parent, and whether it draws
/// (`ModelPart.visible`, after Skin Customization).
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PosedPart {
    /// Which part it is.
    pub part: HumanoidPart,
    /// Its pose relative to its parent.
    pub pose: crate::PartPose,
    /// Whether it draws.
    pub visible: bool,
}

/// The humanoid parts of a model, posed this frame: a fixed-size snapshot, nothing allocated.
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PosedParts {
    /// Each of [`HumanoidPart::ALL`] at its index there: None for a part the model lacks.
    pub parts: [Option<PosedPart>; 12],
}

impl PosedParts {
    /// The parts `pose` gives `ModelPart`'s nine numbers for (x, y, z, x rot, y rot, z rot, x, y,
    /// z scale) and whether they draw; None when the model lacks a head or a body (not a humanoid
    /// model). A part the model lacks, such as a zombie's sleeves, is left out.
    pub fn new(pose: impl Fn(HumanoidPart) -> Option<([f32; 9], bool)>) -> Option<Self> {
        let parts = HumanoidPart::ALL.map(|part| {
            let (v, visible) = pose(part)?;
            let pose = crate::PartPose {
                x: v[0],
                y: v[1],
                z: v[2],
                x_rot: v[3],
                y_rot: v[4],
                z_rot: v[5],
                x_scale: v[6],
                y_scale: v[7],
                z_scale: v[8],
            };
            Some(PosedPart {
                part,
                pose,
                visible,
            })
        });
        let posed = Self { parts };
        (posed.get(HumanoidPart::Head).is_some() && posed.get(HumanoidPart::Body).is_some())
            .then_some(posed)
    }

    /// `part` as posed this frame, if the model has it.
    pub fn get(&self, part: HumanoidPart) -> Option<PosedPart> {
        self.parts[part as usize]
    }

    /// The parts the model has, in [`HumanoidPart::ALL`]'s order.
    pub fn iter(&self) -> impl Iterator<Item = PosedPart> + '_ {
        self.parts.iter().flatten().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_humanoid_needs_a_head_and_a_body_and_keeps_each_parts_pose() {
        let pose = |x: f32| ([x, 0.0, 0.0, 0.5, 0.0, 0.0, 1.0, 1.0, 1.0], true);
        let parts = PosedParts::new(|part| match part {
            HumanoidPart::Head => Some(pose(1.0)),
            HumanoidPart::Body => Some(pose(2.0)),
            HumanoidPart::RightArm => Some(([3.0, 0.0, 0.0, 0.0, 0.25, 0.0, 1.0, 1.0, 1.0], false)),
            _ => None,
        })
        .expect("a head and a body make a humanoid");
        let arm = parts.get(HumanoidPart::RightArm).unwrap();
        assert_eq!(
            (arm.pose.x, arm.pose.y_rot, arm.visible),
            (3.0, 0.25, false)
        );
        assert_eq!(parts.get(HumanoidPart::RightSleeve), None);
        assert_eq!(parts.get(HumanoidPart::Head).unwrap().pose.x_rot, 0.5);
        let order: Vec<_> = parts.iter().map(|p| p.part).collect();
        assert_eq!(
            order,
            [
                HumanoidPart::Head,
                HumanoidPart::Body,
                HumanoidPart::RightArm
            ]
        );
        assert!(PosedParts::new(|part| (part == HumanoidPart::Head).then(|| pose(1.0))).is_none());
        let names: Vec<_> = HumanoidPart::ALL.iter().map(|p| p.model_name()).collect();
        assert_eq!(names.len(), 12);
        assert!(names.contains(&"left_pants") && names.contains(&"hat"));
    }
}
