use super::*;
use crate::{IPayloadContext, PayloadBytes, RegisterPayloadHandlersEvent};

struct Handler;
impl IPayloadHandler for Handler {
    fn handle(&self, _: PayloadBytes, _: &mut dyn IPayloadContext) -> Result<(), String> {
        Ok(())
    }
}
fn handler() -> Arc<dyn IPayloadHandler> {
    Arc::new(Handler)
}

#[test]
fn registrar_views_preserve_defaults_and_common_occupies_both_protocols() {
    let mut registrar = PartRegistrar::new("test");
    {
        let mut event: RegisterPayloadHandlersEvent<'_> =
            (&mut registrar as &mut dyn Registrar).payload_handlers();
        let mut required = event.registrar("1");
        required
            .optional()
            .versioned("2")
            .executes_on(HandlerThread::Network)
            .common_bidirectional(PayloadId("test:both".into()), handler(), Some(handler()));
        required.play_to_server(PayloadId("test:required".into()), handler());
    }
    assert!(registrar.refused.is_empty());
    let declarations: Vec<_> = registrar
        .parts
        .payloads
        .as_ref()
        .unwrap()
        .declarations()
        .collect();
    assert_eq!(declarations.len(), 3);
    let required = declarations
        .iter()
        .find(|entry| entry.id.0 == "test:required")
        .unwrap();
    assert!(!required.optional);
    assert_eq!(required.version, "1");
    assert_eq!(required.thread, HandlerThread::Main);
    for entry in declarations
        .iter()
        .filter(|entry| entry.id.0 == "test:both")
    {
        assert!(entry.optional);
        assert_eq!(entry.version, "2");
        assert_eq!(entry.thread, HandlerThread::Network);
        assert_eq!(entry.direction, PayloadDirection::Bidirectional);
    }
    // A conflicting common declaration refuses atomically: it must not leave
    // behind a configuration entry when the play id is already occupied.
    (&mut registrar as &mut dyn Registrar)
        .payload_handlers()
        .registrar("3")
        .common_to_server(PayloadId("test:required".into()), handler());
    assert_eq!(registrar.refused.len(), 1);
    assert_eq!(
        registrar
            .parts
            .payloads
            .as_ref()
            .unwrap()
            .declarations()
            .count(),
        3
    );
}

#[test]
fn client_event_supplies_both_common_handlers_and_refuses_replacement() {
    let mut registrar = PartRegistrar::new("test");
    let id = PayloadId("test:rules".into());
    (&mut registrar as &mut dyn Registrar)
        .payload_handlers()
        .registrar("1")
        .executes_on(HandlerThread::Network)
        .common_to_client(id.clone(), None);
    assert_eq!(
        registrar
            .parts
            .payloads
            .as_ref()
            .unwrap()
            .validate_client_handlers()
            .unwrap_err()
            .len(),
        2
    );
    (&mut registrar as &mut dyn Registrar)
        .client_payload_handlers()
        .register(&id, handler());
    assert!(
        registrar
            .parts
            .payloads
            .as_ref()
            .unwrap()
            .validate_client_handlers()
            .is_ok()
    );
    (&mut registrar as &mut dyn Registrar)
        .client_payload_handlers()
        .register(&id, handler());
    assert_eq!(registrar.refused.len(), 1);
    assert!(registrar.refused[0].contains("second client payload handler"));
}

#[test]
fn invalid_ownership_versions_and_receiving_handlers_refuse_load() {
    for id in ["bare", "other:stolen", "minecraft:reserved"] {
        let mut registrar = PartRegistrar::new("test");
        (&mut registrar as &mut dyn Registrar)
            .payload_handlers()
            .registrar("1")
            .play_to_server(PayloadId(id.into()), handler());
        assert_eq!(registrar.refused.len(), 1, "{id}");
        assert!(registrar.parts.payloads.is_none());
    }
    let mut registrar = PartRegistrar::new("test");
    (&mut registrar as &mut dyn Registrar)
        .payload_handlers()
        .registrar("  ")
        .play_to_server(PayloadId("test:empty_version".into()), handler());
    assert_eq!(registrar.refused.len(), 1);
    assert!(registrar.parts.payloads.is_none());
    registrar.add_payload(
        PayloadDeclaration {
            id: PayloadId("test:no_handler".into()),
            version: "1".into(),
            protocol: PayloadProtocol::Play,
            direction: PayloadDirection::ToServer,
            optional: false,
            thread: HandlerThread::Main,
        },
        None,
        None,
    );
    assert_eq!(registrar.refused.len(), 2);
    assert!(registrar.parts.payloads.is_none());
}
