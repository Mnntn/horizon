use super::*;

// no wire: frozen registration storage; each boundary value has its own Wire derive.
#[derive(Clone)]
pub(crate) struct MenuBookDeclaration {
    pub(crate) owner: &'static str,
    pub(crate) book: crate::RecipeBookTypeKey,
    pub(crate) recipe_type: String,
    pub(crate) layout: crate::RecipeBookLayout,
    pub(crate) screen: crate::Screen,
}

#[cfg(test)]
#[path = "menus/book_tests.rs"]
mod book_tests;
impl HookParts {
    pub(crate) fn menu<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn MenuPart>] {
        attached(&self.menus, id)
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to the mod menu type `menu`.
    pub fn menu_part<P: MenuPart>(&mut self, menu: &str, part: P) {
        self.attach_menu_part(menu, type_name::<P>(), Arc::new(part));
    }
}

/// Registrar operations for menus.
pub trait MenusRegistrar {
    /// Declare this menu's book, authoritative recipe type and destination roles.
    /// The menu remains content-owned; loading checks these roles against its
    /// actual container slots and crafting declaration. `screen` is the existing
    /// typed screen value, frozen here: mapped slots need finite coordinates and
    /// its book must equal `book`. This screen owns client geometry and server
    /// hidden-slot policy; these menus do not call the dynamic screen hook.
    /// This does not turn display candidates into ingredient predicates.
    /// Invalid ownership, unknown books, duplicate menus or malformed screen
    /// geometry refuse installation, even when the caller ignores the error.
    fn menu_recipe_book(
        &mut self,
        menu: &str,
        book: &crate::RecipeBookTypeKey,
        recipe_type: &str,
        layout: crate::RecipeBookLayout,
        screen: crate::Screen,
    ) -> Result<(), String>;
    /// Attach `part`, of kind `kind`, to the mod menu type `menu`.
    fn attach_menu_part(&mut self, menu: &str, kind: &'static str, part: Arc<dyn MenuPart>);
}

impl MenusRegistrar for PartRegistrar {
    fn menu_recipe_book(
        &mut self,
        menu: &str,
        book: &crate::RecipeBookTypeKey,
        recipe_type: &str,
        layout: crate::RecipeBookLayout,
        screen: crate::Screen,
    ) -> Result<(), String> {
        let result: Result<(), String> = (|| {
            if installed().is_some() {
                return Err("registry is frozen".into());
            }
            let menu_id =
                crate::Identifier::parse(menu).map_err(|e| format!("invalid menu: {e:?}"))?;
            if !menu.contains(':') || !owns_namespace(self.mod_id, menu_id.namespace()) {
                return Err(
                    "menu namespace must be explicit and owned by the registering mod".into(),
                );
            }
            if !self.recipe_book_types.contains_key(book.as_str()) {
                return Err(format!("unknown recipe book {}", book.as_str()));
            }
            if self.menu_recipe_books.contains_key(menu) {
                return Err("duplicate menu recipe book".into());
            }
            crate::Identifier::parse(recipe_type)
                .map_err(|e| format!("invalid recipe type: {e:?}"))?;
            if !recipe_type.contains(':') {
                return Err("explicit recipe type namespace required".into());
            }
            layout.validate(256)?;
            layout.validate_screen(&screen)?;
            if screen.recipe_book.as_ref() != Some(book) {
                return Err("screen book differs from menu book".into());
            }
            self.menu_recipe_books.insert(
                menu.into(),
                MenuBookDeclaration {
                    owner: self.mod_id,
                    book: book.clone(),
                    recipe_type: recipe_type.into(),
                    layout,
                    screen,
                },
            );
            Ok(())
        })();
        result.map_err(|reason| {
            let message = format!("mod {} menu {menu:?}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }
    fn attach_menu_part(&mut self, menu: &str, kind: &'static str, part: Arc<dyn MenuPart>) {
        self.attach(|p| &mut p.menus, kind, menu, part);
    }
}
