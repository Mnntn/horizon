//! Content declaration conversion. Admission retains the content loader's validation.
use super::TryIntoEngine;
#[path = "arm_pose.rs"]
mod arm_pose;
use horizon_mod_package::content as engine;

impl TryIntoEngine<()> for mod_api::BlockProperties {
    type Engine = engine::BlockProperties;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::BlockProperties {
            hardness: self.hardness,
            explosion_resistance: self.explosion_resistance,
            light: self.light,
            requires_tool: self.requires_tool,
            friction: self.friction,
            map_color: self.map_color,
            sounds: self.sounds.clone(),
            random_ticks: self.random_ticks.as_ref().map(|v| match v {
                mod_api::RandomTicks::All(b) => engine::RandomTicks::All(*b),
                mod_api::RandomTicks::States { property, values } => engine::RandomTicks::States {
                    property: property.clone(),
                    values: values.clone(),
                },
            }),
            render_shape: self.render_shape.as_ref().map(|v| match v {
                mod_api::RenderShape::Model => engine::RenderShape::Model,
                mod_api::RenderShape::EntityblockAnimated => {
                    engine::RenderShape::EntityblockAnimated
                }
                mod_api::RenderShape::Invisible => engine::RenderShape::Invisible,
            }),
            push_reaction: self.push_reaction.as_ref().map(|v| match v {
                mod_api::PushReaction::PushPull => engine::PushReaction::PushPull,
                mod_api::PushReaction::Push => engine::PushReaction::Push,
                mod_api::PushReaction::Popped => engine::PushReaction::Popped,
                mod_api::PushReaction::Immoveable => engine::PushReaction::Immoveable,
                mod_api::PushReaction::IgnoreEntity => engine::PushReaction::IgnoreEntity,
            }),
            occludes: self.occludes,
            destroy_overlay: self.destroy_overlay,
        })
    }
}
impl TryIntoEngine<()> for mod_api::BlockShapes {
    type Engine = engine::BlockShapes;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::BlockShapes(engine::StateValues {
            property: String::new(),
            properties: self.properties.clone(),
            values: self.values.clone(),
            default: self.default.clone(),
        }))
    }
}
impl TryIntoEngine<()> for mod_api::ShapeEntityKind {
    type Engine = engine::ShapeEntityKind;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(match self {
            Self::Item => engine::ShapeEntityKind::Item,
            Self::Player => engine::ShapeEntityKind::Player,
            Self::Living => engine::ShapeEntityKind::Living,
            Self::Other => engine::ShapeEntityKind::Other,
        })
    }
}
impl TryIntoEngine<()> for mod_api::Block {
    type Engine = engine::Block;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::Block {
            state_light: None,
            id: self.id.clone(),
            template: self.template.clone(),
            pick_item: self.pick_item.clone(),
            properties: self.properties.try_into_engine(&())?,
            shapes: self
                .shapes
                .as_ref()
                .map(|v| v.try_into_engine(&()))
                .transpose()?,
            collision_shapes: self
                .collision_shapes
                .as_ref()
                .map(|v| v.try_into_engine(&()))
                .transpose()?,
            outline_context_shapes: self
                .outline_context_shapes
                .iter()
                .map(|(kind, shape)| Ok((kind.try_into_engine(&())?, shape.try_into_engine(&())?)))
                .collect::<Result<_, String>>()?,
            collision_context_shapes: self
                .collision_context_shapes
                .iter()
                .map(|(kind, shape)| Ok((kind.try_into_engine(&())?, shape.try_into_engine(&())?)))
                .collect::<Result<_, String>>()?,
            interaction_shapes: self
                .interaction_shapes
                .as_ref()
                .map(|s| s.try_into_engine(&()))
                .transpose()?,
            state_properties: self
                .state_properties
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        match v {
                            mod_api::BlockStateProperty::Boolean => engine::StateProperty::Boolean,
                            mod_api::BlockStateProperty::Enum { values } => {
                                engine::StateProperty::Enum {
                                    values: values.clone(),
                                }
                            }
                        },
                    )
                })
                .collect(),
            default_state: self.default_state.clone(),
            item: self.item,
            open_sound: self.open_sound.clone(),
            close_sound: self.close_sound.clone(),
            block_item: self.block_item.clone(),
            components: component_patch(&self.components)?,
            connects_to: self.connects_to.clone(),
            thickness: self.thickness,
            wood_type: self.wood_type.clone(),
            wall_block: self.wall_block.clone(),
            sign_text_color: self.sign_text_color.clone(),
            sign_text_glow: self.sign_text_glow,
            base_block: self.base_block.clone(),
        })
    }
}
impl TryIntoEngine<()> for mod_api::Item {
    type Engine = engine::Item;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::Item {
            id: self.id.clone(),
            template: self.template.clone(),
            max_stack_size: self.max_stack_size,
            rarity: self.rarity.as_ref().map(|v| match v {
                mod_api::Rarity::Common => engine::Rarity::Common,
                mod_api::Rarity::Uncommon => engine::Rarity::Uncommon,
                mod_api::Rarity::Rare => engine::Rarity::Rare,
                mod_api::Rarity::Epic => engine::Rarity::Epic,
            }),
            components: component_patch(&self.components)?,
            crafting_remainder: self.crafting_remainder.clone(),
            block: self.block.clone(),
            arm_pose: self
                .arm_pose
                .map(|pose| pose.try_into_engine(&()))
                .transpose()?,
        })
    }
}

fn component_patch(
    patch: &[(String, Option<mod_api::Tag>)],
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, String> {
    patch
        .iter()
        .map(|(id, value)| match value {
            Some(value) => horizon_core::json_nbt::tag_to_json(value)
                .map(|value| (id.clone(), value))
                .map_err(|error| error.to_string()),
            None => Ok((format!("!{id}"), serde_json::json!({}))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Block, BlockProperties, Item, Tag};
    use std::collections::BTreeMap;

    #[test]
    fn context_shape_declarations_preserve_categories_and_boxes() {
        let shape = mod_api::BlockShapes {
            properties: vec![],
            values: BTreeMap::new(),
            default: Some(vec![[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]]),
        };
        let mut draft = Block::new("test:shape", "minecraft:stone");
        for kind in [
            mod_api::ShapeEntityKind::Item,
            mod_api::ShapeEntityKind::Player,
            mod_api::ShapeEntityKind::Living,
            mod_api::ShapeEntityKind::Other,
        ] {
            draft.outline_context_shapes.insert(kind, shape.clone());
            draft.collision_context_shapes.insert(kind, shape.clone());
        }
        draft.interaction_shapes = Some(shape);
        let result = draft.try_into_engine(&()).unwrap();
        let boxes = Some(vec![[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]]);
        for kind in [
            engine::ShapeEntityKind::Item,
            engine::ShapeEntityKind::Player,
            engine::ShapeEntityKind::Living,
            engine::ShapeEntityKind::Other,
        ] {
            assert_eq!(result.outline_context_shapes[&kind].default, boxes);
            assert_eq!(result.collision_context_shapes[&kind].default, boxes);
        }
        assert_eq!(result.interaction_shapes.unwrap().default, boxes);
    }

    #[test]
    fn declaration_conversion_preserves_admission_outcomes() {
        // Independent old-path records: errors must match byte for byte, not just reject.
        for (id, template, light, hardness, item) in [
            ("test:block", "minecraft:stone", 15, 2.5, true),
            ("other:block", "minecraft:stone", 0, 1.0, false),
            ("test:Bad", "minecraft:stone", 0, 1.0, false),
            ("test:block", "other:stone", 0, 1.0, false),
            ("test:block", "minecraft:stone", 16, 1.0, false),
            ("test:block", "minecraft:stone", 0, f32::INFINITY, false),
        ] {
            let api = {
                let mut draft = Block::new(id, template);
                draft.item = item;
                draft.properties = BlockProperties {
                    light: Some(light),
                    hardness: Some(hardness),
                    ..Default::default()
                };
                draft
            };
            let old = engine::Block {
                id: id.into(),
                template: template.into(),
                item,
                properties: engine::BlockProperties {
                    light: Some(light),
                    hardness: Some(hardness),
                    ..Default::default()
                },
                ..Default::default()
            };
            let converted = api.try_into_engine(&()).unwrap();
            assert_eq!(converted, old);
            let mut expected = horizon_mod_package::Content::declared("test");
            let mut actual = horizon_mod_package::Content::declared("test");
            assert_eq!(
                actual.declare(&[converted], &[]),
                expected.declare(&[old], &[])
            );
            assert_eq!(actual.blocks, expected.blocks);
            if light == 16 {
                let valid = engine::Block {
                    id: "test:first".into(),
                    template: "minecraft:stone".into(),
                    ..Default::default()
                };
                let bad = api.try_into_engine(&()).unwrap();
                let mut admission = horizon_mod_package::Content::declared("test");
                let error = admission.declare(&[valid, bad], &[]).unwrap_err();
                assert_eq!(error, "block test:block: light is 0 to 15");
                assert!(admission.blocks.is_empty());
                assert!(admission.items.is_empty());
            }
        }
        for (id, template, max, remainder) in [
            ("test:item", "minecraft:stick", 64, None),
            ("other:item", "minecraft:stick", 64, None),
            ("test:item", "other:stick", 64, None),
            ("test:item", "minecraft:stick", 0, None),
            ("test:item", "minecraft:stick", 100, None),
            ("test:item", "minecraft:stick", 64, Some("test:missing")),
            ("test:item", "minecraft:stick", 64, Some("minecraft:bucket")),
        ] {
            let api = {
                let mut draft = Item::new(id, template);
                draft.max_stack_size = Some(max);
                draft.crafting_remainder = remainder.map(str::to_owned);
                draft
            };
            let old = engine::Item {
                id: id.into(),
                template: template.into(),
                max_stack_size: Some(max),
                crafting_remainder: remainder.map(str::to_owned),
                ..Default::default()
            };
            let converted = api.try_into_engine(&()).unwrap();
            assert_eq!(converted, old);
            let mut expected = horizon_mod_package::Content::declared("test");
            let mut actual = horizon_mod_package::Content::declared("test");
            assert_eq!(
                actual.declare(&[], &[converted]),
                expected.declare(&[], &[old])
            );
            assert_eq!(actual.items, expected.items);
        }
    }

    #[test]
    fn declaration_component_patch_preserves_values_and_removals() {
        let api = {
            let mut draft = Item::new("test:item", "minecraft:stick");
            draft.components = vec![
                ("minecraft:max_stack_size".into(), Some(Tag::Int(16))),
                (
                    "minecraft:custom_data".into(),
                    Some(Tag::Compound(
                        [(horizon_core::nbt::JavaString::from("energy"), Tag::Int(42))].into(),
                    )),
                ),
                ("minecraft:damage".into(), None),
            ];
            draft
        };
        let converted = api.try_into_engine(&()).unwrap();
        assert_eq!(
            converted.components,
            BTreeMap::from([
                ("minecraft:max_stack_size".into(), serde_json::json!(16)),
                (
                    "minecraft:custom_data".into(),
                    serde_json::json!({"energy": 42})
                ),
                ("!minecraft:damage".into(), serde_json::json!({})),
            ])
        );
        let mut invalid = api;
        invalid.components = vec![(
            "minecraft:custom_data".into(),
            Some(Tag::Double(f64::NAN.to_bits())),
        )];
        assert!(invalid.try_into_engine(&()).is_err());
    }

    #[test]
    fn declaration_nested_block_overrides_are_not_lost() {
        let api = {
            let mut draft = Block::new("test:block", "minecraft:stone");
            draft.properties = BlockProperties {
                friction: Some(0.8),
                explosion_resistance: Some(7.0),
                requires_tool: Some(true),
                map_color: Some(9),
                sounds: Some("minecraft:glass".into()),
                random_ticks: Some(mod_api::RandomTicks::States {
                    property: "powered".into(),
                    values: vec!["true".into()],
                }),
                render_shape: Some(mod_api::RenderShape::Invisible),
                push_reaction: Some(mod_api::PushReaction::Popped),
                occludes: Some(false),
                destroy_overlay: Some(false),
                ..Default::default()
            };
            draft.state_properties =
                BTreeMap::from([("powered".into(), mod_api::BlockStateProperty::Boolean)]);
            draft.default_state = BTreeMap::from([("powered".into(), "false".into())]);
            draft.shapes = Some(mod_api::BlockShapes {
                properties: vec![],
                values: BTreeMap::new(),
                default: Some(vec![[0.0, 0.0, 0.0, 16.0, 8.0, 16.0]]),
            });
            draft
        };
        let converted = api.try_into_engine(&()).unwrap();
        assert_eq!(
            converted.properties,
            engine::BlockProperties {
                friction: Some(0.8),
                explosion_resistance: Some(7.0),
                requires_tool: Some(true),
                map_color: Some(9),
                sounds: Some("minecraft:glass".into()),
                random_ticks: Some(engine::RandomTicks::States {
                    property: "powered".into(),
                    values: vec!["true".into()]
                }),
                render_shape: Some(engine::RenderShape::Invisible),
                push_reaction: Some(engine::PushReaction::Popped),
                occludes: Some(false),
                destroy_overlay: Some(false),
                ..Default::default()
            }
        );
        assert_eq!(
            converted.state_properties,
            BTreeMap::from([("powered".into(), engine::StateProperty::Boolean)])
        );
        assert_eq!(
            converted.default_state,
            BTreeMap::from([("powered".into(), "false".into())])
        );
        assert_eq!(
            converted.shapes.unwrap().default,
            Some(vec![[0.0, 0.0, 0.0, 16.0, 8.0, 16.0]])
        );
    }
}

pub(super) fn validate_decoded_effects(
    patch: Option<&mod_api::Tag>,
    stack: &horizon_items::items::ItemStack,
) -> Result<(), mod_api::StackWriteError> {
    // The component codec already decoded each supplied value. Check that admission
    // succeeded rather than parsing untrusted component payloads a second time.
    if let Some(patch) = patch
        && ((patch.get("minecraft:consumable").is_some() && stack.consumable(None).is_none())
            || (patch.get("minecraft:death_protection").is_some()
                && stack.death_protection(None).is_none()))
    {
        return Err(mod_api::StackWriteError::InvalidEffectPayload);
    }
    Ok(())
}

#[cfg(test)]
mod effect_write_tests {
    use super::super::{TryIntoEngine, stack_limit_tests::catalog};
    use horizon_core::nbt::JavaString;
    use mod_api::{ItemStack, StackWriteError, Tag};

    #[test]
    fn malformed_effect_override_refuses_typed_stack_write() {
        let items = catalog();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        for (component, effects) in [
            ("minecraft:consumable", "on_consume_effects"),
            ("minecraft:death_protection", "death_effects"),
        ] {
            let invalid = Tag::compound([(
                effects,
                Tag::List(vec![Tag::compound([
                    ("type", Tag::String("example:charge".into())),
                    ("bad", Tag::Double(f64::NAN.to_bits())),
                ])]),
            )]);
            let mut draft = ItemStack::of(key);
            draft
                .set_components([(JavaString::from(component), invalid)].into())
                .unwrap();
            assert_eq!(
                draft.try_into_engine(&items),
                Err(StackWriteError::InvalidEffectPayload)
            );
        }
    }
}

impl TryIntoEngine<()> for (String, mod_api::WorldFluid) {
    type Engine = engine::FluidLook;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        let (id, fluid) = self;
        Ok(engine::FluidLook {
            id: id.clone(),
            sprite: None,
            tint: None,
            world: Some(engine::WorldFluid {
                tick_delay: fluid.tick_delay,
                drop_off: fluid.drop_off,
                slope_find_distance: fluid.slope_find_distance,
                source_conversion: fluid.source_conversion,
                bucket: fluid.bucket,
            }),
        })
    }
}
