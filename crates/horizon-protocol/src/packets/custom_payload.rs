//! Directional discard-fallback limits for configuration and play custom payloads.
//! Existing registered engine channels retain their own codec constraints.
use crate::{
    Error, Result,
    buf::{Codec, Ident, Reader, Writer},
};

/// Exact application bytes following a namespaced channel identifier.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomPayload {
    pub channel: String,
    pub data: Vec<u8>,
}

/// Details of a directional payload size refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadTooLarge {
    pub id: String,
    pub size: usize,
    pub limit: usize,
}

/// ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE in vanilla 26.3.
pub const SERVERBOUND_LIMIT: usize = 32_767;
/// ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE in vanilla 26.3.
pub const CLIENTBOUND_LIMIT: usize = 1_048_576;

/// Common payload with a directional unknown-id limit, excluding the identifier.
/// Registered payloads use their own codec constraints, not this discard cap.
pub struct PayloadCodec<const CLIENTBOUND: bool, const PLAY: bool>;
pub type ServerboundPayload = PayloadCodec<false, false>;
pub type ClientboundPayload = PayloadCodec<true, false>;
pub type ServerboundPlayPayload = PayloadCodec<false, true>;
pub type ClientboundPlayPayload = PayloadCodec<true, true>;

/// The single registered-codec lookup seam. The transport registration work
/// replaces this engine-only classification with its frozen codec lookup.
/// These channels already have engine decoders; they are not discard payloads.
fn uses_registered_payload_codec(channel: &str, play: bool, clientbound: bool) -> bool {
    match channel {
        "minecraft:brand" => true,
        crate::mod_list::CHANNEL => !play && !clientbound,
        "horizon:block_registry" => !play,
        crate::quiet::CHANNEL => {
            if play {
                !clientbound
            } else {
                clientbound
            }
        }
        crate::mod_net::CHANNEL | "horizon:lod" => play || clientbound,
        crate::attachment_sync::CHANNEL
        | crate::menu_extra::CHANNEL
        | crate::anvil_cost::CHANNEL
        | "horizon:recipe_content" => play && clientbound,
        "horizon:recipe_transfer" | "horizon:recipe_book_settings" => play,
        _ => false,
    }
}

impl<const CLIENTBOUND: bool, const PLAY: bool> PayloadCodec<CLIENTBOUND, PLAY> {
    fn check(channel: &str, size: usize) -> Result<()> {
        let limit = if CLIENTBOUND {
            CLIENTBOUND_LIMIT
        } else {
            SERVERBOUND_LIMIT
        };
        if !uses_registered_payload_codec(channel, PLAY, CLIENTBOUND) && size > limit {
            return Err(Error::payload_too_large(channel, size, limit));
        }
        Ok(())
    }

    fn validate_body(channel: &str, body: &[u8]) -> Result<()> {
        if !PLAY
            && CLIENTBOUND
            && matches!(channel, crate::quiet::CHANNEL | crate::mod_net::CHANNEL)
            && !body.is_empty()
        {
            return Err(Error::new(format!(
                "{channel} configuration advertisement must be empty"
            )));
        }
        validate_brand(channel, body)
    }
}

impl<const CLIENTBOUND: bool, const PLAY: bool> Codec<CustomPayload>
    for PayloadCodec<CLIENTBOUND, PLAY>
{
    fn read(r: &mut Reader) -> Result<CustomPayload> {
        let channel = Ident::read(r)?;
        let size = r.remaining();
        Self::check(&channel, size)?;
        let body = r.rest();
        Self::validate_body(&channel, body)?;
        Ok(CustomPayload {
            channel,
            data: body.to_vec(),
        })
    }

    fn validate(v: &CustomPayload) -> Result<()> {
        Self::check(&v.channel, v.data.len())?;
        Self::validate_body(&v.channel, &v.data)
    }

    fn write(v: &CustomPayload, w: &mut Writer) {
        Ident::write(&v.channel, w);
        w.bytes(&v.data);
    }
}

fn validate_brand(channel: &str, body: &[u8]) -> Result<()> {
    if channel == "minecraft:brand" {
        let mut reader = Reader::new(body);
        reader.string(32_767)?;
        if reader.remaining() != 0 {
            return Err(Error::new("brand payload has trailing bytes"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ids,
        packets::{configuration, play},
    };

    #[test]
    fn directional_limits_in_both_states() {
        check(32_767, |b| {
            configuration::Serverbound::decode(ids::configuration::serverbound::CUSTOM_PAYLOAD, b)
                .map(|_| ())
        });
        check(1_048_576, |b| {
            configuration::Clientbound::decode(ids::configuration::clientbound::CUSTOM_PAYLOAD, b)
                .map(|_| ())
        });
        check(32_767, |b| {
            play::Serverbound::decode(ids::play::serverbound::CUSTOM_PAYLOAD, b).map(|_| ())
        });
        check(1_048_576, |b| {
            play::Clientbound::decode(ids::play::clientbound::CUSTOM_PAYLOAD, b).map(|_| ())
        });
    }

    #[test]
    fn refusal_leaves_body_unconsumed_and_legal_bytes_stay_exact() {
        let mut body = Writer::new();
        body.string("test:opaque");
        body.bytes(&vec![0x83; 32_768]);
        let mut reader = Reader::new(&body.data);
        assert!(ServerboundPayload::read(&mut reader).is_err());
        assert_eq!(reader.remaining(), 32_768);
        body.data.pop();
        let packet = ServerboundPayload::read(&mut Reader::new(&body.data)).unwrap();
        assert_eq!(packet.channel, "test:opaque");
        assert_eq!(packet.data, vec![0x83; 32_767]);
    }

    #[test]
    fn registered_codec_seam_exempts_only_existing_engine_ids() {
        // Expected receiving codecs: configuration server/client, then play server/client.
        for (channel, expected) in [
            ("minecraft:brand", [true, true, true, true]),
            ("horizon:quiet_commands", [false, true, true, false]),
            ("horizon:mod_net", [false, true, true, true]),
            ("horizon:mod_list", [true, false, false, false]),
            ("horizon:attachment_sync", [false, false, false, true]),
            ("horizon:menu_extra", [false, false, false, true]),
            ("horizon:anvil_cost", [false, false, false, true]),
            ("horizon:lod", [false, true, true, true]),
            ("horizon:block_registry", [true, true, false, false]),
            ("horizon:recipe_content", [false, false, false, true]),
            ("horizon:recipe_transfer", [false, false, true, true]),
            ("horizon:recipe_book_settings", [false, false, true, true]),
        ] {
            for ((play, clientbound), registered) in
                [(false, false), (false, true), (true, false), (true, true)]
                    .into_iter()
                    .zip(expected)
            {
                assert_eq!(
                    uses_registered_payload_codec(channel, play, clientbound),
                    registered,
                    "{channel} play={play} clientbound={clientbound}"
                );
            }
        }
        for channel in [
            "horizon:not_registered",
            "farmersdelight:flip_skillet",
            "minecraft:unknown",
            "other:mod_net",
        ] {
            assert!(
                !uses_registered_payload_codec(channel, false, false),
                "{channel}"
            );
            assert!(ServerboundPayload::check(channel, 32_768).is_err());
            assert!(ClientboundPayload::check(channel, 1_048_577).is_err());
        }
    }

    #[test]
    fn wrong_phase_and_wrong_direction_use_discard_limits() {
        type Decode = fn(&[u8]) -> Result<()>;
        let cases: [(&str, usize, Decode); 4] = [
            ("horizon:recipe_content", 32_767, |b| {
                configuration::Serverbound::decode(
                    ids::configuration::serverbound::CUSTOM_PAYLOAD,
                    b,
                )
                .map(|_| ())
            }),
            ("horizon:anvil_cost", 1_048_576, |b| {
                configuration::Clientbound::decode(
                    ids::configuration::clientbound::CUSTOM_PAYLOAD,
                    b,
                )
                .map(|_| ())
            }),
            ("horizon:anvil_cost", 32_767, |b| {
                play::Serverbound::decode(ids::play::serverbound::CUSTOM_PAYLOAD, b).map(|_| ())
            }),
            ("horizon:mod_list", 1_048_576, |b| {
                play::Clientbound::decode(ids::play::clientbound::CUSTOM_PAYLOAD, b).map(|_| ())
            }),
        ];
        for (channel, limit, decode) in cases {
            let mut body = Writer::new();
            body.string(channel);
            body.bytes(&vec![0; limit]);
            assert!(decode(&body.data).is_ok());
            body.data.push(0);
            let error = decode(&body.data).unwrap_err();
            assert_eq!(error.payload_limit().unwrap().id, channel);
        }
    }

    #[test]
    fn configuration_feature_advertisements_have_empty_body_codecs() {
        for channel in ["horizon:quiet_commands", "horizon:mod_net"] {
            let mut payload = CustomPayload {
                channel: channel.into(),
                data: vec![],
            };
            assert!(
                configuration::Clientbound::CustomPayload(payload.clone())
                    .try_encode()
                    .is_ok()
            );
            payload.data.push(1);
            assert!(
                configuration::Clientbound::CustomPayload(payload)
                    .try_encode()
                    .unwrap_err()
                    .0
                    .contains("must be empty")
            );
            let mut body = Writer::new();
            body.string(channel);
            body.bytes(&vec![0; 1_048_577]);
            let error = configuration::Clientbound::decode(
                ids::configuration::clientbound::CUSTOM_PAYLOAD,
                &body.data,
            )
            .unwrap_err();
            assert!(error.0.contains("must be empty"));
            assert!(error.payload_limit().is_none());
        }
    }

    #[test]
    fn registered_brand_is_not_bounded_by_discard_capacity() {
        // A legal UTF-8 brand can exceed 32,767 bytes while staying below
        // FriendlyByteBuf's 32,767 UTF-16 code-unit string limit.
        let mut brand = Writer::new();
        brand.string(&"é".repeat(16_384));
        let payload = CustomPayload {
            channel: "minecraft:brand".into(),
            data: brand.data,
        };
        assert!(payload.data.len() > 32_767);
        for packet in [
            configuration::Serverbound::CustomPayload(payload.clone())
                .try_encode()
                .unwrap(),
            play::Serverbound::CustomPayload(payload)
                .try_encode()
                .unwrap(),
        ] {
            let mut reader = Reader::new(&packet);
            reader.varint().unwrap();
            assert!(ServerboundPayload::read(&mut reader).is_ok());
        }
    }

    #[test]
    fn checked_encoding_applies_both_limits_in_both_states() {
        for (limit, encode) in [
            (
                32_767,
                (|p| configuration::Serverbound::CustomPayload(p).try_encode())
                    as fn(CustomPayload) -> Result<Vec<u8>>,
            ),
            (1_048_576, |p| {
                configuration::Clientbound::CustomPayload(p).try_encode()
            }),
            (32_767, |p| play::Serverbound::CustomPayload(p).try_encode()),
            (1_048_576, |p| {
                play::Clientbound::CustomPayload(p).try_encode()
            }),
        ] {
            let mut payload = CustomPayload {
                channel: "test:opaque".into(),
                data: vec![0x83; limit],
            };
            assert!(encode(payload.clone()).is_ok());
            payload.data.push(0);
            assert!(encode(payload).unwrap_err().0.contains("PayloadTooLarge"));
        }
    }

    fn check(limit: usize, decode: impl Fn(&[u8]) -> Result<()>) {
        let mut body = Writer::new();
        body.string("test:opaque");
        body.bytes(&vec![0x83; limit]);
        decode(&body.data).unwrap();
        body.data.push(0);
        let error = decode(&body.data).unwrap_err();
        assert!(error.0.contains("PayloadTooLarge"));
        assert!(error.0.contains("test:opaque"));
        assert!(error.0.contains(&format!("size: {}", limit + 1)));
        assert!(error.0.contains(&format!("limit: {limit}")));
    }
}
