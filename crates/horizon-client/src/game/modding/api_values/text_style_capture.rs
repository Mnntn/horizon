use super::*;

impl TryIntoApi for engine::Style {
    type Api = TextStyle;
    type Error = TextFormError;

    fn try_into_api(&self) -> Result<TextStyle, TextFormError> {
        let span = engine::Span::new("", self.clone());
        Ok(capture(Source::Span(&span))?.style)
    }
}
