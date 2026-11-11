"""Spike glue (fn-replace-spike): copy ONE game method out of its file into a free function of a
patch crate. Text-level on purpose: it shows which rewrites a real generator (syn-based, in
horizon-mod-api-gen) has to make, not how to make them robustly.

Rewrites made here, each one a rule the generator needs:
1. the receiver `&self` / `&mut self` becomes a parameter `this: &Game<'_>` / `&mut Game<'_>`,
   and `self` in the body becomes `this`;
2. the file's own `use` items are copied with `super::` / `crate::` made absolute
   (`horizon_client::game::...`, the game's self-crate alias is already `horizon_client`);
3. a glob import of the function's own module brings its items, which the SDK made `pub`.

usage: extract.py <file> <module path> <fn name> <out.rs>
"""
import re
import sys

path, module, name, out = sys.argv[1:5]
lines = open(path).read().split("\n")
start = next(i for i, l in enumerate(lines) if re.search(rf"\bfn {name}\b", l))
indent = len(lines[start]) - len(lines[start].lstrip())
end = next(i for i in range(start + 1, len(lines)) if lines[i] == " " * indent + "}")
body = "\n".join(l[indent:] for l in lines[start:end + 1])
body = re.sub(r"^pub(\([^)]*\))? fn", "pub fn", body)
body = body.replace("(&mut self", "(this: &mut Game<'_>").replace("(&self", "(this: &Game<'_>")
body = re.sub(r"\bself\b", "this", body)

parent = module.rsplit("::", 1)[0]
uses, taking = [], False
for line in lines[:start]:
    if line.startswith("use ") or taking:
        uses.append(line)
        taking = not line.rstrip().endswith(";")
text = "\n".join(uses)
text = re.sub(r"\bsuper::", f"{parent}::", text).replace("use super::{", f"use {parent}::{{")
text = re.sub(r"\bcrate::", "horizon_client::", text)
header = f"#[allow(unused_imports)]\nuse {module}::*;\n" + ("" if re.search(r"\bGame\b", text) else "use horizon_client::game::Game;\n")
open(out, "w").write(header + text + "\n\n" + body + "\n")
