//! Event bus.
//!
//! Services publish [`Event`]s through the [`EventPublisher`] port. The
//! server streams them (SSE) and the activity feature records them.
//!
//! **Events must never contain secrets** (API keys, passwords, tokens). Keep
//! `data` to identifiers, counts and status.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EventLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub level: EventLevel,
    /// Dotted, lowercase name such as `partner.step.succeeded`.
    pub kind: String,
    pub message: String,
    pub data: Value,
}

impl Event {
    pub fn new(
        at: OffsetDateTime,
        level: EventLevel,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            at,
            level,
            kind: kind.into(),
            message: message.into(),
            data: Value::Null,
        }
    }

    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

/// Port used by services to report progress and results.
pub trait EventPublisher: Send + Sync {
    fn publish(&self, event: Event);
}

/// Publisher that drops everything.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullPublisher;

impl EventPublisher for NullPublisher {
    fn publish(&self, _event: Event) {}
}

/// In-process fan-out. Slow subscribers lose the oldest events
/// (`RecvError::Lagged`) rather than blocking publishers.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    pub const DEFAULT_CAPACITY: usize = 256;

    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn into_publisher(self) -> Arc<dyn EventPublisher> {
        Arc::new(self)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}

impl EventPublisher for EventBus {
    fn publish(&self, event: Event) {
        // An error only means there are no subscribers right now.
        let _ = self.tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str) -> Event {
        Event::new(OffsetDateTime::UNIX_EPOCH, EventLevel::Info, kind, "m")
    }

    #[test]
    fn publish_without_subscribers_is_fine() {
        EventBus::default().publish(ev("a"));
    }

    #[test]
    fn all_subscribers_receive_in_order() {
        let bus = EventBus::default();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish(ev("one"));
        bus.publish(ev("two"));
        for rx in [&mut a, &mut b] {
            assert_eq!(rx.try_recv().unwrap().kind, "one");
            assert_eq!(rx.try_recv().unwrap().kind, "two");
        }
    }

    #[test]
    fn slow_subscriber_lags_instead_of_blocking() {
        let bus = EventBus::new(2);
        let mut rx = bus.subscribe();
        for i in 0..5 {
            bus.publish(ev(&format!("e{i}")));
        }
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
        assert_eq!(rx.try_recv().unwrap().kind, "e3");
    }
}
