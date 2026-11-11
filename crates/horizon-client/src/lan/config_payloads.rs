//! Configuration transport has no player. Its replies share the engine writer queue.
use crate::{
    net::{PreparedPayload, queued_writer::QueuedWriter},
    raw_payloads::{Incoming, Transport},
};
use horizon_protocol::{
    connection::Connection, packets::configuration as cfg, payload_negotiation::Phase,
};
use mod_api::{PayloadId, PayloadSendError};
use std::sync::{Arc, mpsc::Sender};

#[derive(Clone)]
pub struct ConfigSender {
    writer: QueuedWriter,
    transport: Arc<Transport>,
}
impl ConfigSender {
    pub fn framing(&self) -> horizon_protocol::connection::Framing {
        self.writer.framing()
    }
    pub fn payload_transport(&self) -> &Arc<Transport> {
        &self.transport
    }
    pub fn prepare_payload(
        &self,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<PreparedPayload, PayloadSendError> {
        self.writer.prepare_payload(body, id)
    }
    pub(super) fn send_packet(&self, body: &[u8]) -> horizon_protocol::Result<()> {
        self.writer.send(body)
    }
    pub fn disconnect(&self, reason: horizon_core::nbt::Tag) {
        self.transport.close();
        self.writer
            .disconnect(cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode());
    }
}

pub(super) struct ConfigIo<'a> {
    connection: &'a mut Connection,
    pub sender: Option<ConfigSender>,
    events: Sender<super::NetEvent>,
    handed_off: bool,
}
impl<'a> ConfigIo<'a> {
    pub fn new(
        connection: &'a mut Connection,
        transport: Arc<Transport>,
        events: Sender<super::NetEvent>,
    ) -> horizon_protocol::Result<Self> {
        let sender = if transport.local.manifest.entries().is_empty() {
            None
        } else {
            Some(ConfigSender {
                writer: QueuedWriter::new(connection.writer.take())?,
                transport,
            })
        };
        Ok(Self {
            connection,
            sender,
            events,
            handed_off: false,
        })
    }
    pub fn send(&mut self, body: &[u8]) -> horizon_protocol::Result<()> {
        match &self.sender {
            Some(sender) => sender.writer.send(body),
            None => self.connection.send(body),
        }
    }
    pub fn application(&self, payload: cfg::CustomPayload) -> Result<bool, String> {
        let Some(sender) = &self.sender else {
            return Ok(false);
        };
        let Some(epoch) = sender.transport.epoch() else {
            return Ok(false);
        };
        if epoch
            .peer
            .receiver(Phase::Configuration, &payload.channel)
            .is_err()
        {
            return Ok(false);
        }
        let incoming = Incoming::new(&sender.transport, Phase::Configuration, payload)
            .map_err(|e| format!("{e:?}"))?;
        self.events
            .send(super::NetEvent::ConfigurationPayload(
                sender.clone(),
                incoming,
            ))
            .map_err(|_| "server closed".to_owned())?;
        Ok(true)
    }
    pub fn disconnect(&mut self, reason: horizon_core::nbt::Tag) {
        match &self.sender {
            Some(sender) => sender.disconnect(reason),
            None => {
                let _ = self
                    .connection
                    .send(&cfg::Clientbound::Disconnect(cfg::Disconnect { reason }).encode());
            }
        }
    }
    pub fn finish(&mut self) -> Result<(), String> {
        if let Some(sender) = &self.sender {
            self.connection.writer = sender.writer.drain_and_take().map_err(|e| e.to_string())?;
        }
        self.handed_off = true;
        Ok(())
    }
}
impl std::ops::Deref for ConfigIo<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
    }
}
impl std::ops::DerefMut for ConfigIo<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
    }
}
impl Drop for ConfigIo<'_> {
    fn drop(&mut self) {
        if !self.handed_off
            && let Some(sender) = &self.sender
        {
            sender.transport.close();
            if !sender.writer.is_terminal() {
                sender.writer.shutdown();
            }
        }
    }
}
