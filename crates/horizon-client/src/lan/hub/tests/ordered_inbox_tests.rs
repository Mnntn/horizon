//! Ordered Hub/action-stream tests. No prepared resources or game process.
use super::*;

fn with_input_context<T>(run: impl FnOnce(&mut TickCtx<'_>) -> T) -> T {
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    run(&mut TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    })
}

fn publish_packet(hub: &Hub, packet: S) {
    let guest = &hub.guests[0];
    assert!(guest.sender.undrained.handed(0));
    hub.lan
        .shared
        .events
        .send(NetEvent::Packet(guest.conn, Box::new(packet), 0))
        .unwrap();
}

#[test]
fn provider_credit_survives_poll_inbox_and_derived_action_segment() {
    let (mut hub, _out) = hub_with_a_guest();
    let provider = hub.lan.shared.events.clone();
    publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
    let reserved = provider.stats().reserved_bytes;
    assert!(reserved > 0);
    with_input_context(|ctx| {
        drop(hub.poll(ctx));
        assert_eq!(provider.stats().queued_items, 0);
        assert_eq!(provider.stats().reserved_bytes, reserved);
        let events = hub.drain_packets(ctx);
        assert!(events.iter().any(
            |e| matches!(e, HostEvent::Packet(_, packet) if matches!(&**packet, S::Punch(_)))
        ));
        assert!(hub.guests[0].inbox.is_empty());
        assert_eq!(provider.stats().reserved_bytes, reserved);
        drop(events); // Game finished or discarded the complete segment.
        assert_eq!(provider.stats().reserved_bytes, 0);
    });
}

#[test]
fn leaving_physical_guest_keeps_then_discards_its_queued_input_credit() {
    let (mut hub, _out) = hub_with_a_guest();
    let provider = hub.lan.shared.events.clone();
    publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
    let conn = hub.guests[0].conn;
    provider
        .send(NetEvent::Left(conn, "test departure".into()))
        .unwrap();
    with_input_context(|ctx| {
        let events = hub.poll(ctx);
        assert!(hub.guests.is_empty());
        assert!(provider.stats().reserved_bytes > 0);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, HostEvent::Left(g) if g.conn == conn && g.inbox.len() == 1))
        );
        drop(events);
        assert_eq!(provider.stats().reserved_bytes, 0);
    });
}

#[test]
fn derived_action_replies_wait_for_the_next_frozen_receiving_turn() {
    let (mut hub, _out) = hub_with_a_guest();
    let provider = hub.lan.shared.events.clone();
    publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
    with_input_context(|ctx| {
        drop(hub.poll(ctx));
        let events = hub.drain_packets(ctx);
        for event in events {
            if matches!(event, HostEvent::Packet(_, _)) {
                publish_packet(&hub, S::Punch(horizon_protocol::packets::Empty));
            }
        }
        assert_eq!(provider.stats().queued_items, 1);
        assert!(hub.guests[0].inbox.is_empty());
        drop(hub.drain_packets(ctx)); // No new receiving turn, no feedback drain.
        assert_eq!(provider.stats().queued_items, 1);
        drop(hub.poll(ctx));
        drop(hub.drain_packets(ctx));
        assert_eq!(provider.stats().reserved_bytes, 0);
    });
}
