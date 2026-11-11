#!/usr/bin/env python3
"""Extract raw transport turn/credit additions without reordering input application."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/horizon-client/src"

def extract(text, start, end):
    a = text.index(start)
    b = text.index(end, a)
    return text[:a] + text[b:], text[a:b]

# The new resumable receiving turn and its cursor belong together.
p = SRC / "lan/hub.rs"
s = p.read_text()
s, cursor = extract(s, "struct PacketDrain {", "pub struct Hub {")
cursor = cursor.replace("struct PacketDrain", "pub(super) struct PacketDrain")
for field in ["conns", "next", "started", "movement_held"]:
    cursor = cursor.replace(f"    {field}:", f"    pub(super) {field}:")
s, drain = extract(s, "    /// Handle the players in this level's packets", "    /// The level in place's part of the tick")
a = s.index("                NetEvent::RawPayload(delivery) => {")
b = s.index("                NetEvent::Joined(j) => {", a)
raw = s[a:b]
body = raw.split("=> {\n", 1)[1].rsplit("                }", 1)[0]
body = "\n".join(line[12:] if line.startswith("            ") else line for line in body.splitlines())
s = s[:a] + "                NetEvent::RawPayload(delivery) => {\n                    self.poll_raw_payload(delivery, credit, &mut events, &mut credits);\n                }\n" + s[b:]
a = s.index("        if let Some(epoch) = self.payload_player_epoch {")
b = s.index("        guest.last_action_tick", a)
bind = s[a:b]
s = s[:a] + "        self.bind_raw_player(&guest, entity_id);\n" + s[b:]
s = s.replace("mod spawn_info;", "mod spawn_info;\nmod raw_inbox;\nuse raw_inbox::PacketDrain;")
p.write_text(s)
(SRC / "lan/hub/raw_inbox.rs").write_text("//! Ordered raw/ordinary receiving turns and physical input ownership.\nuse super::*;\n\n" + cursor + "impl Hub {\n" + drain + "    pub(super) fn bind_raw_player(&self, guest: &Guest, entity_id: i32) {\n" + bind + "    }\n\n    pub(super) fn poll_raw_payload(\n        &mut self,\n        delivery: Box<crate::raw_payloads::Delivery>,\n        credit: crate::net::ingress::ReceiveLease<NetEvent>,\n        events: &mut Vec<HostEvent>,\n        credits: &mut Vec<crate::net::ingress::ReceiveLease<NetEvent>>,\n    ) {\n" + body + "\n    }\n}\n")

# Retained credit travels with each backlog slot, including decoder placeholders.
p = SRC / "game/network/remote/mod.rs"
s = p.read_text()
s, received = extract(s, "/// The game's backlog, in the order", "/// `Minecraft.services().profileKeySignatureValidator()`") if "/// The game's backlog, in the order" in s else (s, "")
if not received:
    a = s.index("enum Received {")
    # Include only the immediately preceding documentation block.
    a = s.rfind("\n\n", 0, a) + 2
    b = s.index("/// `Minecraft.services().profileKeySignatureValidator()`", a)
    received = s[a:b]
    s = s[:a] + s[b:]
received = received.replace("enum Received {", "pub(super) enum Received {")
s = s.replace("mod recipe_properties;", "mod recipe_properties;\nmod raw_backlog;\nuse raw_backlog::Received;")
p.write_text(s)
(SRC / "game/network/remote/raw_backlog.rs").write_text("//! Transport-owned credit for ordered remote backlog slots.\nuse super::*;\n\n" + received)

# Only the added resume-context block moves; pre-existing event processing stays put.
p = SRC / "game/network/lan/mod.rs"
s = p.read_text()
a = s.index("            // Raw transactions finished above.")
b = s.index("        }\n        if !host.quiet.is_empty()", a)
block = s[a:b]
replacement = "            events = self.resume_raw_inbox(&mut host.hub, entities.as_slice(), spawn);\n"
s = s[:a] + replacement + s[b:]
s = s.replace("mod residency;", "mod residency;\nmod raw_inbox;")
p.write_text(s)
block = "\n".join(line[4:] if line.startswith("    ") else line for line in block.splitlines())
block = block.replace("events = host.hub.drain_packets(&mut ctx);", "hub.drain_packets(&mut ctx)").replace("entities: entities.as_slice(),", "entities,")
(SRC / "game/network/lan/raw_inbox.rs").write_text("//! Reborrow live world state only after a raw root has completed.\nuse super::*;\n\nimpl Game<'_> {\n    pub(super) fn resume_raw_inbox(\n        &mut self,\n        hub: &mut Hub,\n        entities: &[horizon_client::lan::entity::EntityState],\n        spawn: [f64; 3],\n    ) -> Vec<HostEvent> {\n" + block + "\n    }\n}\n")

# Additional small extractions keep all three files at or below their base sizes.
p = SRC / "game/network/lan/mod.rs"
s = p.read_text()
a = s.index("        let mut events = events;\n        loop {")
b = s.index("        if !host.quiet.is_empty()", a)
loop = s[a:b].replace("        let mut events = events;\n", "")
loop = loop.replace("entities.as_slice()", "entities").replace("                return;", "                return None;").replace("                break;", "                return Some(host);")
s = s[:a] + "        let Some(back) = self.drain_raw_turns(host, events, entities.as_slice(), spawn) else {\n            return;\n        };\n        host = back;\n" + s[b:]
p.write_text(s)
p = SRC / "game/network/lan/raw_inbox.rs"
s = p.read_text().replace("entities: entities.as_slice(),", "entities,")
s += "\nimpl Game<'_> {\n    pub(super) fn drain_raw_turns(\n        &mut self,\n        mut host: LanHost,\n        mut events: Vec<HostEvent>,\n        entities: &[horizon_client::lan::entity::EntityState],\n        spawn: [f64; 3],\n    ) -> Option<LanHost> {\n" + loop + "    }\n}\n"
p.write_text(s)
p = SRC / "game/network/remote/mod.rs"
s = p.read_text().replace("                let Received::Event(_, credit) = item else {\n                    unreachable!()\n                };\n                forgotten_credits.push(credit);", "                forgotten_credits.push(item.into_event_credit());")
p.write_text(s)
p = SRC / "game/network/remote/raw_backlog.rs"
s = p.read_text() + "\nimpl Received {\n    pub(super) fn into_event_credit(self) -> horizon_client::net::ingress::ReceiveLease<Event> {\n        let Self::Event(_, credit) = self else {\n            unreachable!()\n        };\n        credit\n    }\n}\n"
p.write_text(s)
p = SRC / "lan/hub.rs"
s = p.read_text()
a = s.index("            // The I/O owner can also close concurrently between packets.")
b = s.index("            if *held", a)
s = s[:a] + "            if self.discard_closed_input(i) {\n                break;\n            }\n" + s[b:]
p.write_text(s)
p = SRC / "lan/hub/raw_inbox.rs"
s = p.read_text() + "\nimpl Hub {\n    pub(super) fn discard_closed_input(&mut self, i: usize) -> bool {\n        // The I/O owner can also close concurrently between packets.\n        if !self.guests[i].left_reason.is_empty()\n            || self.guests[i].sender.payload_transport().closed()\n        {\n            let discarded = self.guests.update(i, |g| std::mem::take(&mut g.inbox));\n            drop(discarded);\n            true\n        } else {\n            false\n        }\n    }\n}\n"
# Reuse the same physical-close check at a resumed raw boundary.
s = s.replace("            if !self.guests[i].left_reason.is_empty()\n                || self.guests[i].sender.payload_transport().closed()\n            {\n                let discarded = self.guests.update(i, |g| std::mem::take(&mut g.inbox));\n                drop(discarded);", "            if self.discard_closed_input(i) {")
p.write_text(s)

# The backlog is an area-root module, discovered rather than hand-listed.
p = SRC / "game/network/remote/mod.rs"
s = p.read_text().replace("mod raw_backlog;\n", "").replace("use raw_backlog::Received;", "use super::raw_backlog::Received;")
p.write_text(s)
p = SRC / "game/network/remote/raw_backlog.rs"
s = p.read_text().replace("use super::*;", "use super::remote_chunks::ChunkFormat;\nuse horizon_client::net::Event;\nuse std::sync::Arc;")
(SRC / "game/network/raw_backlog.rs").write_text(s)
p.unlink()
p = SRC / "lan/hub.rs"
s = p.read_text().replace("use raw_inbox::PacketDrain;", "use raw_inbox::{PacketDrain, yield_raw};")
s = s.replace("                    credits.push(credit);\n                    if !delivery.incoming.epoch.active() {\n                        continue;\n                    }\n                    events.push(HostEvent::RawPayload(delivery));\n                    events.push(HostEvent::InputCredits(credits));\n                    return true;", "                    if yield_raw(delivery, credit, events, &mut credits) {\n                        return true;\n                    }\n                    continue;")
p.write_text(s)
p = SRC / "lan/hub/raw_inbox.rs"
s = p.read_text() + "\npub(super) fn yield_raw(\n    delivery: Box<crate::raw_payloads::Delivery>,\n    credit: crate::net::ingress::ReceiveLease<NetEvent>,\n    events: &mut Vec<HostEvent>,\n    credits: &mut Vec<crate::net::ingress::ReceiveLease<NetEvent>>,\n) -> bool {\n    credits.push(credit);\n    if !delivery.incoming.epoch.active() {\n        return false;\n    }\n    events.push(HostEvent::RawPayload(delivery));\n    events.push(HostEvent::InputCredits(std::mem::take(credits)));\n    true\n}\n"
p.write_text(s)
