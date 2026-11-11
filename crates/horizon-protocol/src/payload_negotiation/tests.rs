use super::*;
fn manifest(optional: bool, version: &str, flow: Flow) -> Manifest {
    let mut b = Manifest::builder();
    b.register(
        RegistrationPhase::Common,
        "test:data",
        flow,
        version,
        optional,
    )
    .unwrap();
    b.finish().unwrap()
}
#[test]
fn idle_and_explicit_barrier() {
    let mut n = Negotiation::new(Manifest::default());
    assert_eq!(n.initial(), None);
    assert_eq!(n.accepted(), None);
    n.finish_without_offer().unwrap();
    assert_eq!(n.accepted(), Some([].as_slice()));
    let mut n = Negotiation::new(manifest(false, "v", Flow::ToClient));
    assert!(matches!(
        n.finish_without_offer(),
        Err(Error::Required {
            reason: Mismatch::Missing,
            ..
        })
    ));
}
#[test]
fn crossed_queries_and_frozen_offer() {
    let m = manifest(false, "v", Flow::Bidirectional);
    let mut a = Negotiation::new(m.clone());
    let mut b = Negotiation::new(m);
    let ao = a.receive(b.initial().unwrap()).unwrap().unwrap();
    let bo = b.receive(a.initial().unwrap()).unwrap().unwrap();
    assert_eq!(a.receive(Control::Query).unwrap(), None);
    assert_eq!(a.receive(bo.clone()).unwrap(), None);
    assert_eq!(b.receive(ao).unwrap(), None);
    assert_eq!(a.accepted().unwrap().len(), 2);
    assert_eq!(a.receive(bo), Err(Error::UnexpectedOffer));
}
#[test]
fn one_sided_and_mismatch_matrix() {
    for optional in [true, false] {
        for peer_optional in [true, false] {
            for (version, flow, reason) in [
                ("other", Flow::ToClient, Mismatch::Version),
                ("v", Flow::ToServer, Mismatch::Direction),
            ] {
                let l = manifest(optional, "v", Flow::ToClient);
                let p = manifest(peer_optional, version, flow);
                let result = intersect(&l, &p);
                if optional && peer_optional {
                    assert_eq!(result.unwrap(), vec![]);
                } else {
                    assert!(
                        matches!(result, Err(Error::Required { reason: r, .. }) if r == reason)
                    );
                }
            }
        }
        let m = manifest(optional, "v", Flow::ToClient);
        for (l, p) in [(&m, &Manifest::default()), (&Manifest::default(), &m)] {
            assert_eq!(intersect(l, p).is_ok(), optional);
        }
        let mut empty = Negotiation::new(Manifest::default());
        assert!(matches!(
            empty.receive(Control::Query).unwrap(),
            Some(Control::Offer(_))
        ));
        assert_eq!(empty.receive(Control::Query).unwrap(), None);
    }
}
#[test]
fn codec_limits_and_malformed() {
    let offer = Control::Offer(manifest(true, &"v".repeat(1000), Flow::ToServer));
    let bytes = offer.encode().unwrap();
    assert_eq!(Control::decode(&bytes).unwrap(), offer);
    assert_eq!(
        Control::decode(&vec![0; MAX_MANIFEST_BYTES + 1]),
        Err(Error::MetadataTooLarge)
    );
    assert_eq!(Control::decode(&[0, 1]), Err(Error::TrailingBytes));
    assert_eq!(Control::decode(&[3]), Err(Error::InvalidFlags));
    let mut w = Writer::new();
    w.u8(1);
    w.varint(4097);
    assert_eq!(Control::decode(&w.data), Err(Error::TooManyRecords));
    let mut w = Writer::new();
    w.u8(1);
    w.varint(1);
    w.u8(0);
    w.varint(98302);
    assert!(matches!(Control::decode(&w.data), Err(Error::Codec(_))));
    let mut w = Writer::new();
    w.u8(1);
    w.varint(2);
    for _ in 0..2 {
        w.u8(0);
        w.string("test:a");
        w.u8(0);
        w.string("v");
        w.u8(1);
    }
    assert!(matches!(
        Control::decode(&w.data),
        Err(Error::Duplicate { .. })
    ));
    let mut invalid = w.data.clone();
    invalid[2] = 7;
    assert_eq!(Control::decode(&invalid), Err(Error::InvalidFlags));
    let mut b = Manifest::builder();
    assert!(matches!(
        b.register(RegistrationPhase::Play, "Bad:id", Flow::ToClient, "v", true),
        Err(Error::InvalidId(_))
    ));
    assert_eq!(
        b.register(
            RegistrationPhase::Play,
            "test:a",
            Flow::ToClient,
            "  ",
            true
        ),
        Err(Error::InvalidVersion)
    );
}
#[test]
fn stable_order_and_long_identifiers() {
    let mut b = Manifest::builder();
    b.register(
        RegistrationPhase::Play,
        &format!("test:{}", "a".repeat(200)),
        Flow::ToServer,
        "v",
        true,
    )
    .unwrap();
    b.register(
        RegistrationPhase::Configuration,
        "test:z",
        Flow::ToClient,
        "v",
        true,
    )
    .unwrap();
    let m = b.finish().unwrap();
    assert_eq!(m.entries()[0].phase(), Phase::Configuration);
    assert_eq!(
        Control::decode(&Control::Offer(m.clone()).encode().unwrap()).unwrap(),
        Control::Offer(m)
    );
}

#[test]
fn builder_bounds_before_storing() {
    let mut b = Manifest::builder();
    for i in 0..MAX_RECORDS {
        b.register(
            RegistrationPhase::Play,
            &format!("test:p{i}"),
            Flow::ToClient,
            "v",
            true,
        )
        .unwrap();
    }
    assert_eq!(
        b.register(
            RegistrationPhase::Play,
            "test:overflow",
            Flow::ToClient,
            "v",
            true
        ),
        Err(Error::TooManyRecords)
    );
    assert_eq!(b.finish().unwrap().entries().len(), MAX_RECORDS);
    let mut b = Manifest::builder();
    let version = "v".repeat(MAX_STRING);
    let mut stored = 0;
    loop {
        match b.register(
            RegistrationPhase::Play,
            &format!("test:p{stored}"),
            Flow::ToClient,
            &version,
            true,
        ) {
            Ok(()) => stored += 1,
            Err(Error::MetadataTooLarge) => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    let m = b.finish().unwrap();
    assert_eq!(m.entries().len(), stored);
    assert!(Control::Offer(m).encode().unwrap().len() <= MAX_MANIFEST_BYTES);
}

#[test]
fn first_offer_replies_and_freezes_without_query() {
    let m = manifest(true, "v", Flow::ToServer);
    let mut n = Negotiation::new(m.clone());
    assert_eq!(
        n.receive(Control::Offer(m.clone())).unwrap(),
        Some(Control::Offer(m))
    );
    assert_eq!(n.accepted().unwrap().len(), 2);
    n.finish_without_offer().unwrap();
    assert_eq!(n.accepted().unwrap().len(), 2);
}
