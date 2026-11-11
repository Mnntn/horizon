//! Nonblocking producers and a blocking consumer sharing a byte budget.
//!
//! Charge each value's owned heap allocations (for a `Vec`, its capacity, not length).
//! The channel adds inline value and queue-node storage to that charge. Allocator
//! metadata is not included, but both allocated bytes and node count are bounded.
//! Values held by callers before sending or after receiving are outside the budget.
use std::collections::LinkedList;
use std::sync::mpsc::{RecvError, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A consistent snapshot of queue pressure, including its lifetime high-water mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    // Includes unpublished permits as well as queued values.
    pub reserved_bytes: usize,
    pub peak_reserved_bytes: usize,
    /// Published values only; unpublished permits are not receiver-visible.
    pub queued_items: usize,
}

/// The capacity cannot hold even one empty value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidCapacity;

/// Refusal returns ownership; callers decide whether to retry or disconnect.
#[derive(Debug, PartialEq, Eq)]
pub enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

/// A nonblocking reservation refusal, before a value exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReserveError {
    Full,
    Disconnected,
}

/// Charged capacity awaiting publication. Dropping it cancels the reservation.
/// A permit is not a producer: the last Sender drop cancels all pending permits.
/// Publication after that or receiver closure returns the supplied value.
pub struct Permit<T> {
    shared: Arc<Shared<T>>,
    charge: usize,
}

struct State<T> {
    pending_bytes: usize,
    queue: LinkedList<(T, usize)>,
    stats: Stats,
    senders: usize,
    receiver_alive: bool,
}
struct Shared<T> {
    capacity: usize,
    state: Mutex<State<T>>,
    ready: Condvar,
}
impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Many producers share the same connection budget.
pub struct Sender<T>(Arc<Shared<T>>);
/// Single consumer; queued values are freed when it is dropped.
pub struct Receiver<T>(Arc<Shared<T>>);

/// Fixed per-message charge, including list links, inline value and byte charge.
/// This is nonzero even for zero-sized values and empty payloads.
pub const fn message_overhead<T>() -> usize {
    size_of::<((T, usize), [usize; 2])>()
}

/// Creates a channel. Capacity is connection pressure, not a per-body protocol cap.
pub fn channel<T>(capacity: usize) -> Result<(Sender<T>, Receiver<T>), InvalidCapacity> {
    if capacity < message_overhead::<T>() {
        return Err(InvalidCapacity);
    }
    let shared = Arc::new(Shared {
        capacity,
        state: Mutex::new(State {
            pending_bytes: 0,
            queue: LinkedList::new(),
            stats: Stats {
                reserved_bytes: 0,
                peak_reserved_bytes: 0,
                queued_items: 0,
            },
            senders: 1,
            receiver_alive: true,
        }),
        ready: Condvar::new(),
    });
    Ok((Sender(shared.clone()), Receiver(shared)))
}

impl<T> Sender<T> {
    /// Never waits for capacity. Overflowing charges are refused as full.
    /// `owned_bytes` must cover all heap storage retained by `value`.
    pub fn try_send(&self, value: T, owned_bytes: usize) -> Result<(), TrySendError<T>> {
        match self.reserve(owned_bytes) {
            Ok(permit) => permit.publish(value).map_err(TrySendError::Disconnected),
            Err(ReserveError::Full) => Err(TrySendError::Full(value)),
            Err(ReserveError::Disconnected) => Err(TrySendError::Disconnected(value)),
        }
    }
    /// Charges owned heap bytes plus inline value and node overhead immediately.
    /// Never waits for capacity. The future value must fit this declared charge.
    pub fn reserve(&self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        let mut state = self.0.lock();
        if !state.receiver_alive {
            return Err(ReserveError::Disconnected);
        }
        let charge = owned_bytes
            .checked_add(message_overhead::<T>())
            .ok_or(ReserveError::Full)?;
        if charge > self.0.capacity - state.stats.reserved_bytes {
            return Err(ReserveError::Full);
        }
        state.pending_bytes += charge;
        state.stats.reserved_bytes += charge;
        state.stats.peak_reserved_bytes = state
            .stats
            .peak_reserved_bytes
            .max(state.stats.reserved_bytes);
        Ok(Permit {
            shared: self.0.clone(),
            charge,
        })
    }
    pub fn stats(&self) -> Stats {
        self.0.lock().stats
    }
}
impl<T> Permit<T> {
    /// Publishes exactly once without charging again. On closure returns ownership.
    pub fn publish(mut self, value: T) -> Result<(), T> {
        let mut state = self.shared.lock();
        if !state.receiver_alive || state.senders == 0 {
            return Err(value);
        }
        state.queue.push_back((value, self.charge));
        state.pending_bytes -= self.charge;
        state.stats.queued_items += 1;
        self.charge = 0;
        drop(state);
        self.shared.ready.notify_one();
        Ok(())
    }
}
impl<T> Drop for Permit<T> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        if state.receiver_alive && state.senders != 0 {
            state.pending_bytes -= self.charge;
            state.stats.reserved_bytes -= self.charge;
        }
    }
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.0.lock().senders += 1;
        Self(self.0.clone())
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.senders -= 1;
        let closed = state.senders == 0;
        if closed {
            state.stats.reserved_bytes -= state.pending_bytes;
            state.pending_bytes = 0;
        }
        drop(state);
        if closed {
            self.0.ready.notify_all();
        }
    }
}
impl<T> Receiver<T> {
    fn pop(state: &mut State<T>) -> Option<T> {
        let (value, charge) = state.queue.pop_front()?;
        state.stats.reserved_bytes -= charge;
        state.stats.queued_items -= 1;
        Some(value)
    }
    pub fn stats(&self) -> Stats {
        self.0.lock().stats
    }
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut state = self.0.lock();
        Self::pop(&mut state).ok_or(if state.senders == 0 {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }
    pub fn recv(&self) -> Result<T, RecvError> {
        let mut state = self.0.lock();
        loop {
            if let Some(value) = Self::pop(&mut state) {
                return Ok(value);
            }
            if state.senders == 0 {
                return Err(RecvError);
            }
            state = self.0.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        let start = Instant::now();
        let mut state = self.0.lock();
        loop {
            if let Some(value) = Self::pop(&mut state) {
                return Ok(value);
            }
            if state.senders == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let Some(remaining) = timeout.checked_sub(start.elapsed()) else {
                return Err(RecvTimeoutError::Timeout);
            };
            state = self
                .0
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let queue = {
            let mut state = self.0.lock();
            state.receiver_alive = false;
            state.pending_bytes = 0;
            state.stats.reserved_bytes = 0;
            state.stats.queued_items = 0;
            std::mem::take(&mut state.queue)
        };
        // Arbitrary value destructors must run outside the lock.
        drop(queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn pending_publication_and_abort_share_budget() {
        let charge = message_overhead::<u32>() + 10;
        let (tx, rx) = channel(charge * 2).unwrap();
        let outer = tx.reserve(10).unwrap();
        let inner = tx.clone().reserve(10).unwrap();
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        assert_eq!(tx.try_send(9, 0), Err(TrySendError::Full(9)));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(tx.stats().reserved_bytes, charge * 2);
        assert_eq!(tx.stats().queued_items, 0);
        drop(inner);
        outer.publish(42).unwrap();
        assert_eq!(tx.stats().reserved_bytes, charge);
        assert_eq!(tx.stats().queued_items, 1);
        assert_eq!(rx.recv(), Ok(42));
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(tx.stats().peak_reserved_bytes, charge * 2);
    }

    #[test]
    fn permits_cancel_on_either_endpoint_close() {
        let (tx, rx) = channel::<u32>(1024).unwrap();
        let first = tx.reserve(10).unwrap();
        let second = tx.reserve(10).unwrap();
        drop(rx);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(first.publish(1), Err(1));
        drop(second);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert!(matches!(tx.reserve(0), Err(ReserveError::Disconnected)));

        let (tx, rx) = channel::<u32>(1024).unwrap();
        tx.try_send(7, 0).unwrap();
        let first = tx.reserve(10).unwrap();
        let second = tx.reserve(10).unwrap();
        drop(tx);
        assert_eq!(rx.stats().reserved_bytes, message_overhead::<u32>());
        assert_eq!(first.publish(1), Err(1));
        drop(second);
        assert_eq!(rx.recv(), Ok(7));
        assert_eq!(rx.recv(), Err(RecvError));
        assert_eq!(rx.stats().reserved_bytes, 0);
    }

    #[test]
    fn concurrent_pending_flood_is_bounded() {
        for bytes in [0, 257] {
            let charge = message_overhead::<Vec<u8>>() + bytes;
            let (tx, rx) = channel::<Vec<u8>>(31 * charge).unwrap();
            let held = std::thread::scope(|scope| {
                let workers: Vec<_> = (0..8)
                    .map(|_| {
                        let tx = tx.clone();
                        scope.spawn(move || {
                            (0..1000)
                                .filter_map(|_| tx.reserve(bytes).ok())
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .flat_map(|worker| worker.join().unwrap())
                    .collect::<Vec<_>>()
            });
            assert_eq!(held.len(), 31);
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
            assert_eq!(tx.stats().reserved_bytes, 31 * charge);
            assert_eq!(tx.stats().peak_reserved_bytes, 31 * charge);
            drop(held);
            assert_eq!(tx.stats().reserved_bytes, 0);
        }
    }

    #[test]
    fn concurrent_flood_bounds_heap_and_empty_messages() {
        for payload in [0, 257] {
            let charge = message_overhead::<Vec<u8>>() + payload;
            let capacity = charge * 31;
            let (tx, rx) = channel(capacity).unwrap();
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let tx = tx.clone();
                    scope.spawn(move || {
                        for _ in 0..10_000 {
                            let value = vec![0; payload];
                            let _ = tx.try_send(value, payload);
                        }
                    });
                }
            });
            assert_eq!(
                tx.stats(),
                Stats {
                    reserved_bytes: capacity,
                    peak_reserved_bytes: capacity,
                    queued_items: 31
                }
            );
            let mut retained_heap = 0;
            for _ in 0..31 {
                retained_heap += rx.try_recv().unwrap().capacity();
            }
            assert_eq!(retained_heap, payload * 31);
            assert_eq!(tx.stats().reserved_bytes, 0);
            tx.try_send(Vec::new(), 0).unwrap();
            assert!(rx.recv().unwrap().is_empty());
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
            assert_eq!(
                tx.try_send(Vec::new(), usize::MAX),
                Err(TrySendError::Full(Vec::new()))
            );
            assert_eq!(tx.stats().reserved_bytes, 0);
        }
    }

    #[test]
    fn disconnect_releases_values_and_budget() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = channel::<Counted>(1024).unwrap();
        assert!(tx.try_send(Counted(drops.clone()), 0).is_ok());
        drop(rx);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert!(matches!(
            tx.try_send(Counted(drops.clone()), 0),
            Err(TrySendError::Disconnected(_))
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(tx.stats().queued_items, 0);
    }

    #[test]
    fn blocking_delivery_timeout_and_last_sender_close() {
        assert!(channel::<()>(0).is_err());
        let (tx, rx) = channel::<u32>(1024).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        );
        std::thread::scope(|scope| {
            scope.spawn(move || {
                tx.try_send(42, 0).unwrap();
            });
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(42));
            assert_eq!(rx.recv(), Err(RecvError));
            assert_eq!(
                rx.recv_timeout(Duration::ZERO),
                Err(RecvTimeoutError::Disconnected)
            );
        });
    }
}
