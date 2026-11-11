use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes,
    PayloadContextError, PayloadDirection, PayloadId, PayloadProtocol, PayloadWork,
    PayloadWorkStatus, Registrar,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

const NAME: &str = "game::payload_transport::config_tests::tcp_configuration_reply_precedes_finish";
static NETWORK_ENTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const CHANNEL: &str = "raw_probe:message";
struct Probe {
    expected: Arc<Vec<u8>>,
    server: Arc<AtomicUsize>,
    client: Arc<AtomicUsize>,
    network: bool,
}
struct Handler {
    expected: Arc<Vec<u8>>,
    hits: Arc<AtomicUsize>,
    server: bool,
    network: bool,
}
struct Work {
    bytes: std::sync::Mutex<Option<PayloadBytes>>,
    hits: Arc<AtomicUsize>,
    server: bool,
    network_thread: std::thread::ThreadId,
}
impl PayloadWork for Work {
    fn run(&self, context: &mut dyn IPayloadContext) -> Result<(), String> {
        assert_ne!(
            std::thread::current().id(),
            self.network_thread,
            "work must actually switch executors"
        );
        assert_eq!(context.protocol(), PayloadProtocol::Configuration);
        assert!(matches!(
            context.player(),
            Err(PayloadContextError::NoPlayer)
        ));
        finish(
            self.bytes.lock().unwrap().take().unwrap(),
            context,
            self.server,
            &self.hits,
        )
    }
}
fn finish(
    bytes: PayloadBytes,
    context: &mut dyn IPayloadContext,
    server: bool,
    hits: &AtomicUsize,
) -> Result<(), String> {
    if server {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        assert_eq!(context.flow(), PayloadDirection::ToServer);
        assert!(context.server_host().is_ok());
        assert!(matches!(
            context.client_host(),
            Err(PayloadContextError::WrongSide)
        ));
        context
            .reply(&PayloadId(CHANNEL.into()), bytes)
            .map_err(|error| format!("reply: {error:?}"))?;
    } else {
        assert_eq!(context.flow(), PayloadDirection::ToClient);
        assert!(context.client_host().is_ok());
        assert!(matches!(
            context.server_host(),
            Err(PayloadContextError::WrongSide)
        ));
    }
    hits.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
impl IPayloadHandler for Handler {
    fn handle(&self, bytes: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        assert_eq!(
            &bytes.0,
            self.expected.as_ref(),
            "exact raw application bytes"
        );
        assert_eq!(context.protocol(), PayloadProtocol::Configuration);
        assert!(matches!(
            context.player(),
            Err(PayloadContextError::NoPlayer)
        ));
        if self.network {
            NETWORK_ENTERED.store(true, Ordering::SeqCst);
            if self.server {
                assert!(matches!(
                    context.server_host(),
                    Err(PayloadContextError::WrongThread)
                ));
                assert!(matches!(
                    context.client_host(),
                    Err(PayloadContextError::WrongSide)
                ));
            } else {
                assert!(matches!(
                    context.client_host(),
                    Err(PayloadContextError::WrongThread)
                ));
                assert!(matches!(
                    context.server_host(),
                    Err(PayloadContextError::WrongSide)
                ));
            }
            assert_eq!(
                context.enqueue_work(Arc::new(Work {
                    bytes: std::sync::Mutex::new(Some(bytes)),
                    hits: self.hits.clone(),
                    server: self.server,
                    network_thread: std::thread::current().id(),
                })),
                Ok(PayloadWorkStatus::Queued)
            );
            Ok(())
        } else {
            finish(bytes, context, self.server, &self.hits)
        }
    }
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(if self.network {
                HandlerThread::Network
            } else {
                HandlerThread::Main
            })
            .configuration_bidirectional(
                PayloadId(CHANNEL.into()),
                Arc::new(Handler {
                    expected: self.expected.clone(),
                    hits: self.server.clone(),
                    server: true,
                    network: self.network,
                }),
                Some(Arc::new(Handler {
                    expected: self.expected.clone(),
                    hits: self.client.clone(),
                    server: false,
                    network: self.network,
                })),
            );
    }
}

// A controlled peer holds known packs while the real server executor replies.
#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES; starts a real integrated server and TCP peer"]
fn tcp_configuration_reply_precedes_finish() {
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "HORIZON_CONFIG_TRANSPORT_CHILD",
        &["main", "network"],
    ) {
        return;
    }
    let network = std::env::var("HORIZON_CONFIG_TRANSPORT_CHILD").unwrap() == "network";
    let expected = Arc::new(
        (0..1 << 20)
            .map(|i| ((i * 31 + 7) % 251) as u8)
            .collect::<Vec<_>>(),
    );
    let hits = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "raw_probe",
        Box::new(Probe {
            expected: expected.clone(),
            server: hits.clone(),
            client: Arc::new(AtomicUsize::new(0)),
            network,
        }),
    )]
    .into();
    mod_api::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            let options = horizon_client::title::WorldOptions {
                world_type: horizon_client::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated(
                "config_transport",
                Some("Configuration transport"),
                Some(1),
                true,
                Some(&options),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.joined)
            {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let port = game
                .publish(super::super::lan::LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                })
                .unwrap();
            controlled_peer(port, &expected, game, network);
            assert_eq!(hits.load(Ordering::SeqCst), 1);
            game.leave_server(None).unwrap();
        });
    });
}

fn controlled_peer(port: u16, expected: &[u8], game: &super::super::Game<'_>, network: bool) {
    use horizon_protocol::{
        connection::Connection,
        packets::{Empty, configuration as cfg, handshake, login, play},
        payload_negotiation::{Control, Flow, Manifest, RegistrationPhase},
    };
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: horizon_protocol::PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: "ConfigPeer".into(),
            uuid: horizon_protocol::crypto::offline_uuid("ConfigPeer"),
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, body) = c.recv().unwrap();
        match login::Clientbound::decode(id, &body).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            p => panic!("unexpected login packet {p:?}"),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();
    let mut manifest = Manifest::builder();
    manifest
        .register(
            RegistrationPhase::Configuration,
            CHANNEL,
            Flow::Bidirectional,
            "1",
            false,
        )
        .unwrap();
    for control in [Control::Query, Control::Offer(manifest.finish().unwrap())] {
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
                data: control.encode().unwrap(),
            })
            .encode(),
        )
        .unwrap();
    }
    let mut known = None;
    let mut offered = false;
    while known.is_none() || !offered {
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::SelectKnownPacks(p) => known = Some(p),
            cfg::Clientbound::CustomPayload(p)
                if p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
            {
                assert!(matches!(
                    Control::decode(&p.data).unwrap(),
                    Control::Offer(_)
                ));
                offered = true;
            }
            cfg::Clientbound::FinishConfiguration(_) => panic!("finished before known packs"),
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    let (release, held) = std::sync::mpsc::channel();
    if network {
        let (entered, ready) = std::sync::mpsc::channel();
        assert!(game.integrated.as_ref().unwrap().post(move |_| {
            entered.send(()).unwrap();
            held.recv_timeout(Duration::from_secs(30)).unwrap();
        }));
        ready.recv_timeout(Duration::from_secs(30)).unwrap();
    }
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: CHANNEL.into(),
            data: expected.to_vec(),
        })
        .encode(),
    )
    .unwrap();
    if network {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !NETWORK_ENTERED.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "NETWORK handler never ran while main was held"
            );
            std::thread::yield_now();
        }
    }
    // Release immediately: the pending MAIN delivery/work must fence FinishConfiguration.
    c.send(&cfg::Serverbound::SelectKnownPacks(known.unwrap()).encode())
        .unwrap();
    if network {
        release.send(()).unwrap();
    }
    let mut replies = 0;
    loop {
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                assert_eq!(p.data, expected);
                replies += 1;
                assert_eq!(replies, 1);
            }
            cfg::Clientbound::CustomPayload(p) => assert_ne!(p.channel, "horizon:mod_net"),
            cfg::Clientbound::FinishConfiguration(_) => {
                assert_eq!(replies, 1);
                break;
            }
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    // Receiving the play login proves the actual connection handed off successfully.
    loop {
        let (id, body) = c.recv().unwrap();
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::Login(_) => break,
            play::Clientbound::CustomPayload(p) => {
                assert_ne!(p.channel, CHANNEL, "configuration reply leaked into play")
            }
            _ => {}
        }
    }
}
