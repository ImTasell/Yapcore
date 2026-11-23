use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use thiserror::Error;

use crate::commands::HandlerResult;
use crate::models::EventEnvelope;

/// Stage at which a bot runtime or user handler failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorStage {
    /// Event dispatch failed before or during built-in handling.
    EventDispatch,
    /// A generic event listener returned an error.
    EventListener,
    /// A prefix command handler returned an error.
    CommandHandler,
    /// A slash command handler returned an error.
    SlashCommandHandler,
    /// A slash command handler failed after automatic acknowledgement.
    SlashCommandAfterAck,
    /// A button handler returned an error.
    ButtonHandler,
    /// An error listener itself returned an error.
    ErrorHandler,
}

/// Context for an error observed by the bot runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorEvent {
    /// Runtime stage that produced the error.
    pub stage: ErrorStage,
    /// Yapper event cursor, when the error came from an event.
    pub event_id: Option<u64>,
    /// Original event type, when available.
    pub event_type: Option<String>,
    /// Registered command name, when available.
    pub command: Option<String>,
    /// Button custom ID, when available.
    pub custom_id: Option<String>,
    /// Human-readable error message.
    pub message: String,
}

impl ErrorEvent {
    pub(crate) fn new(stage: ErrorStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            event_id: None,
            event_type: None,
            command: None,
            custom_id: None,
            message: message.into(),
        }
    }
}

type ErrorFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send + 'static>>;
pub(crate) type ErrorHandler = Arc<dyn Fn(ErrorEvent) -> ErrorFuture + Send + Sync + 'static>;

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
pub(crate) type EventHandler = Arc<dyn Fn(EventEnvelope) -> HandlerFuture + Send + Sync + 'static>;

/// Ordered event-listener registry.
#[derive(Default)]
pub(crate) struct EventRegistry {
    handlers: BTreeMap<String, Vec<EventHandler>>,
    error_handlers: Vec<ErrorHandler>,
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
            .field("error_handlers", &self.error_handlers.len())
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

    /// Register an asynchronous lifecycle error listener.
    pub fn register_error<F, Fut>(&mut self, handler: F)
    where
        F: Fn(ErrorEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.error_handlers
            .push(Arc::new(move |event| Box::pin(handler(event))));
    }

    /// Clone error listeners in registration order.
    pub fn error_handlers(&self) -> Vec<ErrorHandler> {
        self.error_handlers.clone()
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
