#!/usr/bin/env python3
"""Remove private injection source staging; listed slots remain."""
from pathlib import Path

def change(p, f):
 p=Path(p); p.write_text(f(p.read_text()))
def block(s, start):
 a=s.index(start); b=s.index('{',a); depth=1; i=b+1
 while depth:
  depth += (s[i]=='{')-(s[i]=='}'); i+=1
 return s[:a]+s[i:]
g='crates/horizon-mod-api-gen/src/'
change(g+'build.rs',lambda s:s.replace('    let private = super::points::source::build_input(root)?;\n','').replace('        &private,\n',''))
change(g+'points.rs',lambda s: block(s[s.index(''):] .replace('pub mod source;',''), '    let private = source::read(root)?') if False else s)
# Point records no longer include unpublished private IDs.
p=Path(g+'points.rs'); s=p.read_text().replace('pub mod source;',''); a=s.index('    let private = source::read(root)?'); b=s.index('    let (scanned, list)',a); s=s[:a]+s[b:]; s=block(s,'        if private.contains'); s=s.replace(', sources: &[source::Point]','').replace('records_selected(points, sources, None)','records_selected(points, None)').replace('    sources: &[source::Point],\n',''); a=s.index('    let private_variants:'); b=s.index('    let ids:',a); s=s[:a]+s[b:]; s=s.replace(' #(#private_variants)*','').replace('            &source::build_input(&root).unwrap(),\n',''); p.write_text(s)
change(g+'sites/stage.rs',lambda s:block(block(s,'    if private.exists()'), '    names.extend(').replace('    let private = root.join(super::source::FILE);\n','').replace('let mut names =','let names ='))
# Keep private argument/record policy unchanged for the incoming listed-record change.
change(g+'sites/rewrite.rs',lambda s:block(block(s,'                if let Some(expression)'), '                if let Some(local)').replace('crate::points::source::eligible(attrs, sig)','point_shape::eligible(attrs, sig)'))
# Move deterministic facade naming into the listed-site module.
src=Path(g+'sites/source.rs').read_text(); a=src.index('/// The sole facade'); b=src.index('/// Resolve every request',a); facade=src[a:b].replace('super::rewrite::name','rewrite::name')
change(g+'sites.rs',lambda s:s.replace('pub mod source;',facade).replace('pub mod expression;','').replace('mod local;',''))
# The removed machinery has no runtime callers.
change(g+'lib.rs',lambda s:s.replace('pub mod fields;',''))
for name in ['points/source.rs','sites/source.rs','sites/expression.rs','sites/local.rs','fields.rs']:
 Path(g+name).unlink()
import shutil
shutil.rmtree(g+'fields')
# CLI only selects compiled functions and calls.
c='crates/cargo-horizon-mod/src/'
change(c+'inject.rs',lambda s:s.replace('&& if request.kind == Kind::Field {\n                    i.kind == "struct"\n                } else {\n                    matches!(i.kind.as_str(), "fn" | "method")\n                }','&& matches!(i.kind.as_str(), "fn" | "method")').replace(' | Kind::Field | Kind::Expression',''))
change(c+'inject_sync.rs',lambda s:s.replace('    manifest::{MANIFEST, inject::Kind},','    manifest::MANIFEST,').replace('if entry.kind == Kind::Field {\n                format!("type {}", entry.at)\n            } else if','if'))
change(c+'inject_update.rs',lambda s:s.replace('let kind = if inject.kind == Kind::Field {\n        "struct"\n    } else {\n        "fn"\n    };','let kind = "fn";'))
change(c+'wasm.rs',lambda s:block(block(s.replace('entry.kind == horizon_mod_package::manifest::inject::Kind::Field\n            || entry.access','entry.access'),'    if entry.kind == horizon_mod_package::manifest::inject::Kind::Field'), '    #[test]\n    fn field_wasm_refuses_with_attachments'))
