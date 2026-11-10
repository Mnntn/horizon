use super::*;
impl Game<'_> {
    pub(in crate::game) fn text_width(&self, text: &horizon_client::text::Text) -> i32 {
        self.gui.atlas.font.width_text(text) as i32
    }

    pub(in crate::game) fn text_split(
        &self,
        text: &horizon_client::text::Text,
        width: i32,
    ) -> Vec<horizon_client::text::Text> {
        self.gui
            .atlas
            .font
            .split_formatted(&text.resolved(), width as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::TryIntoEngine;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn styled_text_reads_use_live_font_language_and_wrapping_outside_drawing() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let mut text = mod_api::Text::literal("AA AA");
            text.style.bold = Some(true);
            let form = mod_api::text::TextForm::try_from(&text).unwrap();
            let text = mod_api::Text::try_from(form).unwrap();
            let host: &dyn mod_api::ClientHost = game;
            assert_eq!(host.text_width(&mod_api::Text::literal("AA")).unwrap(), 12);
            assert_eq!(host.text_width(&mod_api::Text::literal("")).unwrap(), 0);
            let lines = host.text_split(&text, 14).unwrap();
            let plain: Vec<_> = lines
                .iter()
                .map(|line| {
                    let line: horizon_client::text::Text = line.try_into_engine(&()).unwrap();
                    assert!(line.chars().all(|(_, style)| style.is_bold()));
                    line.plain()
                })
                .collect();
            assert_eq!(plain, ["AA", "AA"]);
            assert_eq!(
                host.text_split(&mod_api::Text::literal("AA"), -1)
                    .unwrap()
                    .len(),
                2
            );

            let lines = host
                .text_split(&mod_api::Text::literal("A\nAA"), 100)
                .unwrap();
            let plain: Vec<_> = lines
                .iter()
                .map(|line| {
                    let line: horizon_client::text::Text = line.try_into_engine(&()).unwrap();
                    line.plain()
                })
                .collect();
            assert_eq!(plain, ["A", "AA"]);

            let mut invalid = mod_api::Text::literal("A");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                let mut parent = mod_api::Text::literal("");
                parent.children.push(invalid);
                invalid = parent;
            }
            assert_eq!(
                host.text_width(&invalid),
                Err(mod_api::TextFormError::TooDeep)
            );
            assert!(matches!(
                host.text_split(&invalid, 100),
                Err(mod_api::TextFormError::TooDeep)
            ));

            let translated = mod_api::Text::new(mod_api::text::TextContent::Translatable {
                key: "test.measure".into(),
                fallback: None,
                arguments: vec![],
            });
            for (value, width, lines) in [("A", 6, 1), ("AA", 12, 2)] {
                horizon_client::lang::scoped(
                    horizon_client::lang::Language::of(
                        "test",
                        [("test.measure".into(), value.into())],
                    ),
                    || {
                        assert_eq!(host.text_width(&translated).unwrap(), width);
                        assert_eq!(host.text_split(&translated, 6).unwrap().len(), lines);
                    },
                );
            }
            // A live replacement font uses explicit provider advances, not any
            // metrics cached by a previous host read.
            let source = horizon_assets::resource_source::ResourceSource::indexed(
                std::collections::BTreeMap::from([(
                    "assets/minecraft/font/default.json".into(),
                    horizon_assets::resource_source::Resource::bytes(
                        br#"{"providers":[{"type":"space","advances":{"A":4}}]}"#.to_vec(),
                    ),
                )]),
            );
            game.gui.atlas.font = horizon_client::font::Font::metrics(&source);
            let host: &dyn mod_api::ClientHost = game;
            assert_eq!(host.text_width(&mod_api::Text::literal("AA")).unwrap(), 8);
            assert_eq!(
                host.text_split(&mod_api::Text::literal("AA"), 8)
                    .unwrap()
                    .len(),
                1
            );
        });
    }
}
