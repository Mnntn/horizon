#!/usr/bin/env python3
"""Thread explicit frozen payload-codec context through packet readers and encoders.

Run once from the repository root. Separate behavioral implementations and tests
follow this mechanical plumbing commit; no legacy mod API is deleted here.
"""
from pathlib import Path


def replace(path, old, new):
    file = Path(path)
    text = file.read_text()
    assert text.count(old) == 1, (path, old)
    file.write_text(text.replace(old, new))


replace("crates/horizon-protocol/src/buf.rs", "    pos: usize,\n}", "    pos: usize,\n    payload_codecs: Option<&'a crate::packets::custom_payload::PayloadCodecs>,\n}")
replace("crates/horizon-protocol/src/buf.rs", "        Self { data, pos: 0 }", "        Self { data, pos: 0, payload_codecs: None }")
replace("crates/horizon-protocol/src/buf.rs", "    pub fn remaining(&self) -> usize {", """    /// Read using this connection's frozen custom-payload codecs.
    pub fn with_payload_codecs(data: &'a [u8], codecs: &'a crate::packets::custom_payload::PayloadCodecs) -> Self {
        Self { data, pos: 0, payload_codecs: Some(codecs) }
    }
    pub(crate) fn payload_codecs(&self) -> Option<&crate::packets::custom_payload::PayloadCodecs> {
        self.payload_codecs
    }
    pub fn remaining(&self) -> usize {""")
replace("crates/horizon-protocol/src/buf.rs", "    fn validate(_v: &T) -> Result<()> {\n        Ok(())\n    }", """    fn validate(_v: &T) -> Result<()> {
        Ok(())
    }
    /// Validate using this connection's frozen custom-payload codecs.
    fn validate_with_payload_codecs(v: &T, _codecs: &crate::packets::custom_payload::PayloadCodecs) -> Result<()> {
        Self::validate(v)
    }""")
replace("crates/horizon-protocol/src/packets/mod.rs", "                use $ids as ids;\n                let mut r = $crate::buf::Reader::new(payload);\n                let packet = match id {", """                Self::decode_reader(id, $crate::buf::Reader::new(payload))
            }
            /// Decode with the local receiving codecs frozen at mod load.
            pub fn decode_with_payload_codecs(id: i32, payload: &[u8], codecs: &$crate::packets::custom_payload::PayloadCodecs) -> $crate::Result<Self> {
                Self::decode_reader(id, $crate::buf::Reader::with_payload_codecs(payload, codecs))
            }
            fn decode_reader(id: i32, mut r: $crate::buf::Reader<'_>) -> $crate::Result<Self> {
                use $ids as ids;
                let packet = match id {""")
replace("crates/horizon-protocol/src/packets/mod.rs", "_ => return Ok(Self::Unknown(id, payload.to_vec())),", "_ => return Ok(Self::Unknown(id, r.rest().to_vec())),")
replace("crates/horizon-protocol/src/packets/mod.rs", "            /// The packet body: VarInt id then payload, for trusted values.", """            /// Validate registered custom payloads in this connection's codec context.
            pub fn try_encode_with_payload_codecs(&self, codecs: &$crate::packets::custom_payload::PayloadCodecs) -> $crate::Result<Vec<u8>> {
                match self {
                    $(Self::$variant(p) => <$crate::packet_codec!($ty $(, $codec)?) as $crate::buf::Codec<$ty>>::validate_with_payload_codecs(p, codecs)?,)*
                    Self::Unknown(_, _) => {}
                }
                Ok(self.encode())
            }
            /// The packet body: VarInt id then payload, for trusted values.""")
