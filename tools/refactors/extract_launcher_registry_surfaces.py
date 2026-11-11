#!/usr/bin/env python3
"""Move launcher surfaces out of ratcheted files; preserve item bodies verbatim."""
from pathlib import Path


def move(source, destination, start, end, prefix, suffix, glue):
    path = Path(source)
    text = path.read_text()
    first = text.index(start)
    last = text.index(end, first)
    item = text[first:last]
    target = Path(destination)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(prefix + item + suffix)
    path.write_text(text[:first] + text[last:] + glue)


move('launcher/src/backend.rs', 'launcher/src/backend/messages.rs',
     'pub enum Msg {', '/// A running process;', 'use super::*;\n\n', '',
     '\nmod messages;\npub use messages::Msg;\n')
move('launcher/src/cli.rs', 'launcher/src/cli/settings_output.rs',
     'fn settings_json(', '/// Runs a subcommand', 'use super::*;\n\n', '',
     '\nmod settings_output;\nuse settings_output::settings_json;\n')
move('launcher/src/main.rs', 'launcher/src/native_build.rs',
     "    /// Builds the selected instance's native mods,", '    fn fetch_releases(',
     'use super::*;\n\nimpl Launcher {\n', '}\n', '\nmod native_build;\n')
