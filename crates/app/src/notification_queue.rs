use service_api::{Notification, NotificationClass};
use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use tokio::sync::Notify;

/// Per-class queue policy is keyed off this trait so unit tests can use a
/// mock type whose classification is set by the test instead of being driven
/// by the wire format.
pub trait Classifiable {
    fn classify(&self) -> NotificationClass;
}

impl Classifiable for Notification {
    fn classify(&self) -> NotificationClass {
        self.class()
    }
}

/// Single ordered notification queue with per-class enqueue policy:
///
/// * `Coalesce { key }` - latest-wins replacement of the existing entry with
///   the same key, preserving its slot in the queue (cross-class FIFO is
///   maintained). If no existing entry matches and the queue is full, the
///   new entry is dropped.
/// * `Drop` - drop oldest under queue pressure. Always appends; pops the
///   front when at capacity.
/// * `MustDeliver` - never coalesced or dropped. Producer awaits when the
///   queue is full so backpressure flows back through the pipe.
///
/// Single consumer is assumed: `recv` pops from the front under the same
/// mutex that `enqueue` uses to mutate the deque.
pub struct NotificationQueue<T: Classifiable = Notification> {
    state: Mutex<QueueState<T>>,
    item_available: Notify,
    space_available: Notify,
    capacity: usize,
}

struct QueueState<T> {
    items: VecDeque<T>,
    closed: bool,
}

impl<T: Classifiable> NotificationQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(QueueState {
                items: VecDeque::new(),
                closed: false,
            }),
            item_available: Notify::new(),
            space_available: Notify::new(),
            capacity,
        }
    }

    pub async fn enqueue(&self, item: T) {
        match item.classify() {
            NotificationClass::Coalesce { key } => self.enqueue_coalesce(&key, item),
            NotificationClass::Drop => self.enqueue_drop(item),
            NotificationClass::MustDeliver => self.enqueue_must_deliver(item).await,
        }
    }

    fn enqueue_coalesce(&self, key: &service_api::CoalesceKey, item: T) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return;
        }
        for slot in &mut state.items {
            if let NotificationClass::Coalesce { key: existing } = slot.classify()
                && &existing == key
            {
                *slot = item;
                self.item_available.notify_one();
                return;
            }
        }
        if state.items.len() >= self.capacity {
            return;
        }
        state.items.push_back(item);
        self.item_available.notify_one();
    }

    fn enqueue_drop(&self, item: T) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return;
        }
        if state.items.len() >= self.capacity {
            state.items.pop_front();
        }
        state.items.push_back(item);
        self.item_available.notify_one();
    }

    async fn enqueue_must_deliver(&self, item: T) {
        let mut pending = Some(item);
        loop {
            let waiter = self.space_available.notified();
            tokio::pin!(waiter);
            {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.closed {
                    return;
                }
                if state.items.len() < self.capacity {
                    state
                        .items
                        .push_back(pending.take().expect("pending was just verified to be Some"));
                    self.item_available.notify_one();
                    return;
                }
            }
            waiter.as_mut().await;
        }
    }

    pub async fn recv(&self) -> Option<T> {
        loop {
            let waiter = self.item_available.notified();
            tokio::pin!(waiter);
            {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(item) = state.items.pop_front() {
                    self.space_available.notify_waiters();
                    return Some(item);
                }
                if state.closed {
                    return None;
                }
            }
            waiter.as_mut().await;
        }
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.closed = true;
        self.item_available.notify_waiters();
        self.space_available.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_api::CoalesceKey;
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Mock {
        class: NotificationClass,
        id: u32,
    }

    impl Classifiable for Mock {
        fn classify(&self) -> NotificationClass {
            self.class.clone()
        }
    }

    fn coalesce_with(key: &str, id: u32) -> Mock {
        Mock {
            class: NotificationClass::Coalesce {
                key: CoalesceKey::test(key),
            },
            id,
        }
    }

    fn drop_with(id: u32) -> Mock {
        Mock {
            class: NotificationClass::Drop,
            id,
        }
    }

    fn must_deliver_with(id: u32) -> Mock {
        Mock {
            class: NotificationClass::MustDeliver,
            id,
        }
    }

    #[tokio::test]
    async fn coalesce_replaces_existing_entry_for_same_key() {
        let queue: NotificationQueue<Mock> = NotificationQueue::new(8);
        queue.enqueue(coalesce_with("k", 1)).await;
        queue.enqueue(coalesce_with("k", 2)).await;
        queue.enqueue(coalesce_with("k", 3)).await;
        assert_eq!(queue.recv().await.map(|m| m.id), Some(3));
        queue.close();
        assert_eq!(queue.recv().await, None);
    }

    #[tokio::test]
    async fn coalesce_preserves_slot_when_replacing() {
        let queue: NotificationQueue<Mock> = NotificationQueue::new(8);
        queue.enqueue(coalesce_with("a", 1)).await;
        queue.enqueue(coalesce_with("b", 2)).await;
        queue.enqueue(coalesce_with("a", 3)).await;
        assert_eq!(queue.recv().await.map(|m| m.id), Some(3));
        assert_eq!(queue.recv().await.map(|m| m.id), Some(2));
        queue.close();
        assert_eq!(queue.recv().await, None);
    }

    #[tokio::test]
    async fn drop_class_evicts_oldest_under_pressure() {
        let queue: NotificationQueue<Mock> = NotificationQueue::new(2);
        queue.enqueue(drop_with(1)).await;
        queue.enqueue(drop_with(2)).await;
        queue.enqueue(drop_with(3)).await;
        assert_eq!(queue.recv().await.map(|m| m.id), Some(2));
        assert_eq!(queue.recv().await.map(|m| m.id), Some(3));
        queue.close();
        assert_eq!(queue.recv().await, None);
    }

    #[tokio::test]
    async fn must_deliver_blocks_producer_when_full() {
        let queue: Arc<NotificationQueue<Mock>> = Arc::new(NotificationQueue::new(1));
        queue.enqueue(must_deliver_with(1)).await;
        let q2 = Arc::clone(&queue);
        let producer = tokio::spawn(async move {
            q2.enqueue(must_deliver_with(2)).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!producer.is_finished());
        assert_eq!(queue.recv().await.map(|m| m.id), Some(1));
        producer.await.expect("producer task panicked");
        assert_eq!(queue.recv().await.map(|m| m.id), Some(2));
        queue.close();
    }

    #[tokio::test]
    async fn close_unblocks_recv_with_none() {
        let queue: Arc<NotificationQueue<Mock>> = Arc::new(NotificationQueue::new(8));
        let q2 = Arc::clone(&queue);
        let consumer = tokio::spawn(async move { q2.recv().await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        queue.close();
        assert!(consumer.await.expect("consumer task panicked").is_none());
    }

    #[tokio::test]
    async fn cross_class_fifo_is_preserved() {
        let queue: NotificationQueue<Mock> = NotificationQueue::new(8);
        queue.enqueue(drop_with(1)).await;
        queue.enqueue(coalesce_with("k", 2)).await;
        queue.enqueue(must_deliver_with(3)).await;
        queue.enqueue(coalesce_with("k", 4)).await;
        assert_eq!(queue.recv().await.map(|m| m.id), Some(1));
        assert_eq!(queue.recv().await.map(|m| m.id), Some(4));
        assert_eq!(queue.recv().await.map(|m| m.id), Some(3));
        queue.close();
        assert_eq!(queue.recv().await, None);
    }

    /// End-to-end smoke for the per-phase coalesce contract using real
    /// `Notification::BootProgress` payloads (the discrepancies-doc test
    /// asked specifically for queue-level coverage with actual wire types,
    /// not just the synthetic Mock harness above). Layout:
    ///   Migrating(1, 10) -> LoadingKey -> Migrating(5, 10) -> OpeningDatabase
    /// expected order out:
    ///   Migrating(5, 10), LoadingKey, OpeningDatabase
    /// Migrating(1, 10) is replaced in-slot by Migrating(5, 10) (per-phase
    /// `BootPhaseKind::Migrating` coalesce key); LoadingKey and
    /// OpeningDatabase have distinct `BootPhaseKind`s so they remain as
    /// independent entries that retain their wire arrival order. A
    /// regression that collapsed every BootProgress under a single
    /// CoalesceKey would surface here as LoadingKey vanishing into the
    /// latest Migrating frame.
    #[tokio::test]
    async fn boot_progress_per_phase_coalesce_keeps_other_phases_independent() {
        use service_api::{BootPhase, BootProgress};

        fn boot_progress(phase: BootPhase) -> Notification {
            Notification::BootProgress(BootProgress {
                phase,
                message: None,
                service_generation: 0,
            })
        }

        let queue: NotificationQueue<Notification> = NotificationQueue::new(16);
        queue
            .enqueue(boot_progress(BootPhase::Migrating { current: 1, total: 10 }))
            .await;
        queue.enqueue(boot_progress(BootPhase::LoadingKey)).await;
        queue
            .enqueue(boot_progress(BootPhase::Migrating { current: 5, total: 10 }))
            .await;
        queue.enqueue(boot_progress(BootPhase::OpeningDatabase)).await;

        // Migrating(1, 10) was replaced in-slot by Migrating(5, 10), so the
        // first slot now carries the latter. LoadingKey and OpeningDatabase
        // remain as independent entries in the order they arrived.
        let phases: Vec<BootPhase> = {
            let mut collected = Vec::new();
            for _ in 0..3 {
                match queue.recv().await.expect("recv") {
                    Notification::BootProgress(p) => collected.push(p.phase),
                    other => panic!("expected BootProgress; got {other:?}"),
                }
            }
            collected
        };
        assert_eq!(
            phases,
            vec![
                BootPhase::Migrating { current: 5, total: 10 },
                BootPhase::LoadingKey,
                BootPhase::OpeningDatabase,
            ],
            "per-phase coalesce must collapse only Migrating; LoadingKey and \
             OpeningDatabase must remain as ordered independent entries"
        );

        queue.close();
        assert!(queue.recv().await.is_none());
    }
}
