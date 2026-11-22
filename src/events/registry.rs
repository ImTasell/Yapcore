use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use thiserror::Error;

use crate::commands::HandlerResult;
use crate::models::EventEnvelope;

/// Invalid event-listener registration details.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventRegistrationError {
    /// Event names must use lowercase ASCII letters, numbers, and underscores.
    #[error("event names must contain only lowercase ASCII letters, numbers, and underscores")]
    InvalidEventName,
    /// Event names cannot exceed 64 characters.
    #[error("event names must not exceed 64 characters")]
    EventNameTooLong,
}

type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send + 'static>>;
pub(crate) type EventHandler =
    Arc<dyn Fn(EventEnvelope) -> HandlerFuture + Send + Sync + 'static>;

/// Ordered event-listener registry.
#[derive(Default)]
pub(crate) struct EventRegistry {
    handlers: BTreeMap<String, Vec<EventHandler>>,
}

impl std::fmt::Debug for EventRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let counts = self
            .handlers
            .iter()
            .map(|(name, handlers)| (name, handlers.len()))
            .collect::<Vec<_>>();

        formatter
            .debug_struct("EventRegistry")
            .field("event_handlers", &counts)
            .finish()
    }
}

impl EventRegistry {
    /// Register an asynchronous listener for one Yapper event type.
    pub fn register<F, Fut>(
        &mut self,
        event_type: impl Into<String>,
        handler: F,
    ) -> Result<(), EventRegistrationError>
    where
        F: Fn(EventEnvelope) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        let event_type = event_type.into();

        if event_type.chars().count() > 64 {
            return Err(EventRegistrationError::EventNameTooLong);
        }

        if event_type.is_empty()
            || !event_type
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(EventRegistrationError::InvalidEventName);
        }

        let handler: EventHandler = Arc::new(move |event| Box::pin(handler(event)));
        self.handlers.entry(event_type).or_default().push(handler);
        Ok(())
    }

    /// Return handlers for an event in registration order.
    pub fn handlers_for(&self, event_type: &str) -> Vec<EventHandler> {
        self.handlers.get(event_type).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_event_names() {
        let mut registry = EventRegistry::default();

        assert_eq!(
            registry.register("", |_event| async { Ok(()) }),
            Err(EventRegistrationError::InvalidEventName)
        );
        assert_eq!(
            registry.register("MemberJoin", |_event| async { Ok(()) }),
            Err(EventRegistrationError::InvalidEventName)
        );
        assert_eq!(
            registry.register("x".repeat(65), |_event| async { Ok(()) }),
            Err(EventRegistrationError::EventNameTooLong)
        );
    }

    #[test]
    fn preserves_multiple_handlers_in_registration_order() {
        let mut registry = EventRegistry::default();

        registry
            .register("member_join", |_event| async { Ok(()) })
            .unwrap();
        registry
            .register("member_join", |_event| async { Ok(()) })
            .unwrap();
        registry
            .register("reaction_add", |_event| async { Ok(()) })
            .unwrap();

        assert_eq!(registry.handlers_for("member_join").len(), 2);
        assert_eq!(registry.handlers_for("reaction_add").len(), 1);
        assert!(registry.handlers_for("unknown").is_empty());
    }
}
