#!/usr/bin/env python3
"""Move the mod surface's arm-pose vocabulary out of horizon-core."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
api = root / 'crates/horizon-mod-api/src'
source = (root / 'crates/horizon-core/src/arm_pose.rs').read_text()
source = source.replace('// exhaustive: HumanoidModel.ArmPose\'s constants; NeoForge\'s enum extensions have no counterpart', '// exhaustive: the vanilla pose vocabulary')
source = source.replace('#[derive(Clone, Copy,', '#[derive(horizon_mod_api_macros::Wire)]\n#[derive(Clone, Copy,', 1)
(api / 'client/arm_pose.rs').write_text(source)
p = api / 'client/mod.rs'
p.write_text(p.read_text() + '\npub(crate) mod arm_pose;\n')
(api / 'exports/arm_pose.rs').write_text('pub use crate::client::arm_pose::ArmPose;\n')
p = api / 'exports/horizon_core.rs'
p.write_text(p.read_text().replace('arm_pose::ArmPose, ', ''))
p = api / 'content/content_declarations.rs'
p.write_text(p.read_text().replace('use horizon_core::arm_pose::ArmPose;', 'use crate::ArmPose;'))
p = api / 'wire/wire_types.rs'
s = p.read_text()
a = s.index('impl crate::wire::Wire for crate::ArmPose {')
b = s.index('impl crate::wire::Wire for crate::UseAnimation {', a)
p.write_text(s[:a] + s[b:])
