#!/usr/bin/env python3
"""Extract developer-launch policy from oversized game and launcher modules.

Run once from the repository root on the developer-offline implementation.
The moved GUI handler's early continues become returns: the original match
is the final statement of the message loop, so both advance to the next message.
"""
from pathlib import Path
import textwrap


def cut(text, start, end):
    assert text.count(start) == 1, start
    begin = text.index(start)
    finish = text.index(end, begin)
    return text[:begin] + text[finish:], text[begin:finish]


def save(path, text):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


game_path = Path('crates/horizon-client/src/game.rs')
game = game_path.read_text()
game, read = cut(game, '    fn read(mut o: Self)', '    /// The command line alone: no file is read and nothing changes.')
read = read.replace('    fn read(', '    pub(super) fn read(', 1)
game, validation = cut(game, '        if o.developer_single_player\n', '        Ok(Some(o))')
game = game.replace('        Ok(Some(o))', '        o.validate_developer_single_player()?;\n        Ok(Some(o))', 1)
save(game_path, game)

lan_path = Path('crates/horizon-client/src/game/network/lan/mod.rs')
lan = lan_path.read_text()
lan_doc = '    /// Start the network side and the play-state hub: for the LAN (with the\n    /// host\'s own profile reserved) or for a dedicated server (no host).\n'
assert lan.count(lan_doc) == 1
lan = lan.replace(lan_doc, '', 1)
lan = lan.replace('    pub(super) fn lan_host(', '    pub(super) fn start_lan_hub(', 1)
lan_guard = '''        if self.options.developer_single_player && net.listen {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
'''
assert lan.count(lan_guard) == 1
lan = lan.replace(lan_guard, '', 1)
save(lan_path, lan)

remote_path = Path('crates/horizon-client/src/game/network/remote/mod.rs')
remote = remote_path.read_text()
remote, local = cut(remote, '    /// Join single player\'s integrated server through memory.\n', '    fn join_server(')
remote, header = cut(remote, '    fn join_server(', '        if local.is_none() && self.options.developer_single_player {')
remote_guard = '''        if local.is_none() && self.options.developer_single_player {
            return Err(
                horizon_client::lang::get("horizon.developer_offline.single_player").into(),
            );
        }
'''
assert remote.count(remote_guard) == 1
remote = remote.replace(remote_guard, header.replace('    fn join_server(', '    pub(super) fn connect_remote_session(', 1), 1)
save(remote_path, remote)

policy = '//! Developer single-player startup and network-entry policy.\nuse super::*;\nuse super::lan::LanSettings;\nuse horizon_client::lan::{NetConfig, hub::HubConfig};\n\nimpl Options {\n'
policy += read
policy += '    pub(super) fn validate_developer_single_player(&self) -> Result<()> {\n'
policy += validation.replace('o.', 'self.')
policy += '        Ok(())\n    }\n}\n\nimpl Game<\'_> {\n'
policy += lan_doc + '''    pub(super) fn lan_host(
        &mut self,
        settings: LanSettings,
        net: NetConfig,
        config: HubConfig,
    ) -> Result<()> {
''' + lan_guard + '''        self.start_lan_hub(settings, net, config)
    }
'''
policy += local
policy += header.replace('    fn join_server(', '    pub(super) fn join_server(', 1)
policy += remote_guard + '''        self.connect_remote_session(address, name, local, cookies, lan, realm)
    }
}
'''
save('crates/horizon-client/src/game/interface/developer_launch.rs', policy)

backend_path = Path('launcher/src/backend.rs')
backend = backend_path.read_text()
backend, client = cut(backend, '/// [`client`], playing as `who` for this launch only (`play --account`).\n', '/// Start the game straight into a friend\'s world:')
backend = backend.replace('/// Downloads `release`\'s archive for this platform', 'mod developer_play;\npub use developer_play::client_as;\n\n/// Downloads `release`\'s archive for this platform', 1)
save(backend_path, backend)
save('launcher/src/backend/developer_play.rs', '//! Account and unsigned-developer policy for CLI Play.\nuse super::*;\n\n' + client)

main_path = Path('launcher/src/main.rs')
main = main_path.read_text()
main, arm = cut(main, '                Msg::Session(session) => {\n', '                Msg::JarConsent')
assert arm.endswith('                }\n')
body = arm.removeprefix('                Msg::Session(session) => {\n').removesuffix('                }\n')
assert body.count('continue;') == 3
body = textwrap.indent(textwrap.dedent(body).replace('continue;', 'return;'), '        ')
main = main.replace('                Msg::JarConsent', '                Msg::Session(session) => self.handle_game_session(session),\n                Msg::JarConsent', 1)
main = main.replace('mod datapacks_page;', 'mod datapacks_page;\nmod developer_play;', 1)
save(main_path, main)
save('launcher/src/developer_play.rs', '''//! Applying account or unsigned-developer sessions to GUI Play.
use super::*;

impl Launcher {
    pub(super) fn handle_game_session(
        &mut self,
        session: std::result::Result<Option<account::GameSession>, account::NoOwner>,
    ) {
''' + body + '    }\n}\n')
print('Extracted game policy, CLI Play and GUI session handling.')
