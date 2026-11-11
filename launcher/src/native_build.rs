use super::*;

impl Launcher {
    /// Builds the selected instance's native mods, once the player accepted the warning. The
    /// build stays on the Horizon the instance plays (`native_mods::base_for`), unless the
    /// warning was Update's: then it moves to that newer Horizon.
    pub(super) fn build_native(&mut self) {
        if self.native_step.is_some() {
            return;
        }
        self.pending_consent = None;
        self.native_error = None;
        // This attempt's own log, starting empty: an open panel shows it rather than the last
        // attempt's (`native_log_page`).
        self.native_log_reset();
        let instance = instances::selected();
        self.native_for = Some(instance.id.clone());
        let plays = self.version_of(&instance).cloned();
        let base = native_mods::base_for(plays.as_deref(), &self.installed, false);
        let Some(base) = base else {
            let error = tr("Install Horizon first.").to_string();
            self.native_log_preflight(&error);
            self.native_error = Some(error);
            return;
        };
        let folder = match self.native_mods_to_build() {
            Ok((folder, _)) => folder,
            Err(e) => {
                self.native_log_preflight(&e);
                self.native_error = Some(e);
                return;
            }
        };
        self.native_step = Some(tr("Starting the build").into());
        self.native_progress = None;
        self.native_done = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = native_mods::build(
                &base,
                &folder,
                &instance,
                None,
                |step| {
                    let _ = tx.send(Msg::NativeProgress(step.label(), step.progress()));
                },
                |inventory| {
                    let (reply, answer) = std::sync::mpsc::channel();
                    tx.send(Msg::NativeRegistryConsent(inventory.clone(), reply))
                        .is_ok()
                        && answer.recv().unwrap_or(false)
                },
            );
            let _ = tx.send(Msg::Native(result));
        });
    }
}
