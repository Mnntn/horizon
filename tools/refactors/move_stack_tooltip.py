from pathlib import Path
p = Path("crates/horizon-client/src/game/blocks/stations.rs")
s = p.read_text()
a = s.index("    /// A stack's tooltip:")
b = s.index("    /// `ClientBundleTooltip.extractSelectedItemTooltip`", a)
Path("crates/horizon-client/src/game/blocks/stations/tooltip.rs").write_text("use super::*;\n\nimpl Game<'_> {\n" + s[a:b] + "}\n")
s = s[:a] + s[b:]
s = s.replace('mod anvil;', 'mod anvil;\nmod tooltip;')
p.write_text(s)
