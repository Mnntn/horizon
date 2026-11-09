#!/usr/bin/env python3
"""Restore the engine trade model from its pre-swap API source, unchanged.

Only the module preamble changes: the engine names its own stack and item table.
The API snapshot is edited separately; this script does not copy its future API
record into the engine or preserve an API compatibility alias.
"""
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
BASE = '483e1ff91d'
SOURCE = 'crates/horizon-mod-api/src/item/merchant.rs'
original = subprocess.check_output(['git', 'show', f'{BASE}:{SOURCE}'], cwd=ROOT, text=True)
old = '''//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
use horizon_items::items::Items;
'''
new = '''//! Engine trading values and payment operations.
use crate::items::{ItemStack, Items};
'''
assert original.count(old) == 1
output = ROOT / 'crates/horizon-client/src/merchant/offer.rs'
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(original.replace(old, new))
parent = ROOT / 'crates/horizon-client/src/merchant.rs'
text = parent.read_text()
old_use = 'pub use mod_api::item::merchant::{ItemCost, MerchantOffer};'
assert text.count(old_use) == 1
parent.write_text(text.replace(old_use, 'mod offer;\npub use offer::{ItemCost, MerchantOffer};'))
print('Restored engine offer operations; only module imports changed')
