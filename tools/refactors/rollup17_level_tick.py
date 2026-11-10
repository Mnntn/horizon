"""Extract the client tick body verbatim for boundary failure testing."""
from pathlib import Path
p = Path('crates/horizon-client/src/game/network/remote/client_level_tick.rs')
s = p.read_text()
a = s.index('        self.ticks += 1;')
b = s.index('        // `ClientLevel.tick`\'s RETURN', a)
body = s[a:b]
body = body.replace('        let result = self.tick_remote_player(running, hold);\n', '        self.tick_remote_player(running, hold)\n')
s = s[:a] + '        let result = body(self);\n' + s[b:]
s = s.replace('    pub(in crate::game) fn tick_remote_level(&mut self) -> Result<()> {', '''    pub(in crate::game) fn tick_remote_level(&mut self) -> Result<()> {
        self.tick_remote_level_with(Self::tick_remote_level_body)
    }

    pub(in crate::game) fn tick_remote_level_with(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {''')
s = s[:-2] + '\n    fn tick_remote_level_body(&mut self) -> Result<()> {\n' + body + '    }\n}\n'
p.write_text(s)
