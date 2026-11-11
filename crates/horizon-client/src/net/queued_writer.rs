//! One bounded writer owner. Receiving callbacks never block their own receive loop
//! while a large reply waits for the peer to drain its socket or memory pipe.
use horizon_mod_runtime::byte_channel::{self, Permit, ReserveError, Sender, TrySendError};
use horizon_protocol::{
    Error, Result,
    adapter::ServerVersion,
    connection::{Closer, ConnWriter, Framing},
    frame::FrameSize,
};
use mod_api::{PayloadFrameStage, PayloadId, PayloadSendError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use std::thread::JoinHandle;

const CAPACITY: usize = 16 * 1024 * 1024;
// Covers the pending transaction journal record as well as the queue's own node.
const PUBLICATION_OVERHEAD: usize = 512;
enum WriteJob {
    Packet(Vec<u8>),
    Contents(Vec<u8>),
    Disconnect(Vec<u8>),
}

#[derive(Clone)]
pub(crate) struct QueuedWriter(Arc<Inner>);
pub(crate) struct Inner {
    sender: Mutex<Option<Sender<WriteJob>>>,
    worker: Mutex<Option<JoinHandle<ConnWriter>>>,
    framing: Framing,
    closer: Arc<Closer>,
    alive: Arc<AtomicBool>,
    retired: AtomicBool,
    terminal: AtomicBool,
    version: ServerVersion,
    secure_chat: Box<dyn Fn() -> Option<bool> + Send + Sync>,
}
impl std::ops::Deref for QueuedWriter {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// Prepared once, charged immediately, invisible to the writer until root commit.
pub struct PreparedPayload {
    permit: Permit<WriteJob>,
    contents: Vec<u8>,
    alive: Arc<AtomicBool>,
}
impl PreparedPayload {
    pub fn publish(self) -> std::result::Result<(), PayloadSendError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(PayloadSendError::Disconnected);
        }
        self.permit
            .publish(WriteJob::Contents(self.contents))
            .map_err(|_| PayloadSendError::Disconnected)
    }
}
impl QueuedWriter {
    pub(crate) fn new(mut writer: ConnWriter) -> Result<Self> {
        let framing = writer.framing();
        let closer = Arc::new(writer.closer()?);
        let alive = Arc::new(AtomicBool::new(true));
        let version = writer.server_version();
        let secure_chat = Box::new(writer.legacy_secure_chat_observer());
        let (sender, receiver) = byte_channel::channel(CAPACITY).expect("nonzero writer capacity");
        let running = alive.clone();
        let worker = std::thread::Builder::new()
            .name("horizon-net-write".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let disconnect = matches!(&job, WriteJob::Disconnect(_));
                    let result = match job {
                        WriteJob::Packet(body) | WriteJob::Disconnect(body) => writer.send(&body),
                        WriteJob::Contents(contents) => writer.send_contents(&contents),
                    };
                    if result.is_err() || disconnect {
                        running.store(false, Ordering::Release);
                        writer.shutdown();
                        break;
                    }
                }
                writer
            })
            .map_err(|error| Error::new(format!("cannot start client writer: {error}")))?;
        Ok(Self(Arc::new(Inner {
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
            retired: AtomicBool::new(false),
            terminal: AtomicBool::new(false),
            framing,
            closer,
            alive,
            version,
            secure_chat,
        })))
    }
    /// Stop acceptance, cancel unpublished reservations and drain published writes.
    pub(crate) fn drain_and_take(&self) -> Result<ConnWriter> {
        let mut sender = self.sender.lock().unwrap();
        self.retired.store(true, Ordering::Release);
        sender.take();
        drop(sender);
        let worker = self
            .worker
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| Error::new("writer already taken"))?;
        let writer = worker.join().map_err(|_| Error::new("writer panicked"))?;
        if !self.alive.swap(false, Ordering::AcqRel) {
            return Err(Error::new("writer disconnected"));
        }
        Ok(writer)
    }
    pub(crate) fn is_terminal(&self) -> bool {
        self.terminal.load(Ordering::Acquire)
    }
    pub(crate) fn framing(&self) -> Framing {
        self.framing
    }
    pub(crate) fn server_version(&self) -> ServerVersion {
        self.version
    }
    pub(crate) fn legacy_secure_chat_enforcement(&self) -> Option<bool> {
        (self.secure_chat)()
    }
    pub(crate) fn shutdown(&self) {
        if self.retired.load(Ordering::Acquire) {
            return;
        }
        self.alive.store(false, Ordering::Release);
        if !self.retired.load(Ordering::Acquire) {
            self.closer.close();
        }
        self.sender.lock().unwrap().take();
    }
    pub(crate) fn send(&self, body: &[u8]) -> Result<()> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(Error::new("client writer disconnected"));
        }
        let owned = body.to_vec();
        let capacity = owned.capacity();
        let sender = self.sender.lock().unwrap();
        let Some(sender) = sender.as_ref() else {
            return Err(Error::new("writer retired"));
        };
        match sender.try_send(WriteJob::Packet(owned), capacity) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.alive.store(false, Ordering::Release);
                self.closer.close();
                Err(Error::new("client writer backpressure"))
            }
            Err(TrySendError::Disconnected(_)) => Err(Error::new("client writer disconnected")),
        }
    }
    pub(crate) fn disconnect(&self, body: Vec<u8>) {
        let mut sender = self.sender.lock().unwrap();
        self.terminal.store(true, Ordering::Release);
        let Some(queue) = sender.take() else {
            return;
        };
        let bytes = body.capacity();
        if queue.try_send(WriteJob::Disconnect(body), bytes).is_err() {
            self.closer.close();
        }
    }
    pub(crate) fn prepare_payload(
        &self,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> std::result::Result<PreparedPayload, PayloadSendError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(PayloadSendError::Disconnected);
        }
        let contents = self.framing.contents_owned(body).map_err(|error| {
            if let Some(detail) = error.frame_limit() {
                PayloadSendError::FrameTooLarge {
                    id: id.clone(),
                    stage: match detail.stage {
                        FrameSize::Encoded => PayloadFrameStage::Encoded,
                        FrameSize::Uncompressed => PayloadFrameStage::Uncompressed,
                    },
                    size: detail.size as u64,
                    limit: detail.limit as u64,
                }
            } else {
                PayloadSendError::EncodeFailed(error.to_string())
            }
        })?;
        let sender = self.sender.lock().unwrap();
        let permit = sender
            .as_ref()
            .ok_or(PayloadSendError::Disconnected)?
            .reserve(contents.capacity() + PUBLICATION_OVERHEAD)
            .map_err(|error| match error {
                ReserveError::Full => PayloadSendError::Backpressure,
                ReserveError::Disconnected => PayloadSendError::Disconnected,
            })?;
        Ok(PreparedPayload {
            permit,
            contents,
            alive: self.alive.clone(),
        })
    }
}
#[cfg(test)]
mod tests;

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.retired.load(Ordering::Acquire) && !self.terminal.load(Ordering::Acquire) {
            self.closer.close();
        }
    }
}
