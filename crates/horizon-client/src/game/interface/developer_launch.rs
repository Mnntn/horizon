//! Developer single-player startup and network-entry policy.
use super::lan::LanSettings;
use super::*;
use horizon_client::lan::{NetConfig, hub::HubConfig};

impl Options {
    pub(super) fn read(mut o: Self) -> Result<Self> {
        if o.pregen {
            horizon_client::command::vanilla::set_horizon_commands(true);
        }
        // The launcher's account: online-mode servers need it. A bad file
        // only leaves the game offline.
        let session = if o.developer_single_player {
            Ok(None)
        } else if o.session_stdin {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| e.to_string())
                .and_then(|_| horizon_client::auth::AuthSession::parse(&text))
                .map(Some)
        } else {
            o.session_file
                .take()
                .filter(|p| !p.is_empty())
                .map(|path| horizon_client::auth::AuthSession::load(path.as_ref()))
                .transpose()
        };
        match session {
            Ok(Some(session)) => {
                o.player_name = Some(session.name.clone());
                horizon_client::auth::set_current(session);
                // The chat key pair, ready before the first join.
                horizon_client::auth::prepare_chat_keys(true);
            }
            Ok(None) => {}
            Err(e) => eprintln!("Horizon: not signed in: {e}"),
        }
        if !o.developer_single_player {
            split::launcher::read_now();
        }
        if let Some(dir) = o.game_dir.take() {
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::env::set_current_dir(&dir))
                .map_err(|e| format!("game folder {}: {e}", dir.display()))?;
        }
        // Vanilla's quick play (Main, QuickPlay.joinSingleplayerWorld): only a
        // world that exists opens; else the title screen. --world also creates one.
        if let Some(world) = o.quick_play.take() {
            if std::env::current_dir()?
                .join("saves")
                .join(&world)
                .join("level.dat")
                .is_file()
            {
                o.world = Some(world);
            } else {
                eprintln!("Quick play: there is no world {world} in saves/");
            }
        }
        Ok(o)
    }
    pub(super) fn validate_developer_single_player(&self) -> Result<()> {
        if self.developer_single_player
            && (self.server.is_some()
                || self.session_stdin
                || self.session_file.is_some()
                || !self.execution_role.is_client())
        {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
        Ok(())
    }
}

impl Game<'_> {
    /// Start the network side and the play-state hub: for the LAN (with the
    /// host's own profile reserved) or for a dedicated server (no host).
    pub(super) fn lan_host(
        &mut self,
        settings: LanSettings,
        net: NetConfig,
        config: HubConfig,
    ) -> Result<()> {
        if self.options.developer_single_player && net.listen {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
        self.start_lan_hub(settings, net, config)
    }
    /// Join single player's integrated server through memory.
    pub(super) fn connect_local(
        &mut self,
        pipe: horizon_protocol::connection::Pipe,
        name: &str,
    ) -> Result<()> {
        self.join_server("local", name, Some(pipe), None, false, false)
    }
    pub(super) fn join_server(
        &mut self,
        address: &str,
        name: &str,
        local: Option<horizon_protocol::connection::Pipe>,
        cookies: Option<horizon_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        if local.is_none() && self.options.developer_single_player {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
        self.connect_remote_session(address, name, local, cookies, lan, realm)
    }
}
