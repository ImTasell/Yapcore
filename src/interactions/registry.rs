use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use thiserror::Error;

use crate::commands::HandlerResult;
use crate::interactions::InteractionContext;

/// Invalid registration details for a component interaction handler.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum InteractionRegistrationError {
    /// A custom ID or prefix cannot be empty.
    #[error("interaction custom IDs and prefixes must not be empty")]
    EmptyCustomId,
    /// A custom ID or prefix exceeds Yapper's 100-character limit.
    #[error("interaction custom IDs and prefixes must not exceed 100 characters")]
    CustomIdTooLong,
}

type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send + 'static>>;
pub(crate) type ButtonHandler =
    Arc<dyn Fn(InteractionContext) -> HandlerFuture + Send + Sync + 'static>;

#[derive(Debug, Clone)]
enum ButtonMatcher {
    Exact(String),
    Prefix(String),
}

struct RegisteredButton {
    matcher: ButtonMatcher,
    handler: ButtonHandler,
}

impl std::fmt::Debug for RegisteredButton {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisteredButton")
            .field("matcher", &self.matcher)
            .finish_non_exhaustive()
    }
}

/// Ordered registry of button/component interaction handlers.
#[derive(Default)]
pub(crate) struct ButtonRegistry {
    entries: Vec<RegisteredButton>,
}

impl std::fmt::Debug for ButtonRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ButtonRegistry")
            .field("handler_count", &self.entries.len())
            .finish()
    }
}

impl ButtonRegistry {
    /// Register a handler for one exact custom ID.
    pub fn register_exact<F, Fut>(
        &mut self,
        custom_id: impl Into<String>,
        handler: F,
    ) -> Result<(), InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.register(ButtonMatcher::Exact(custom_id.into()), handler)
    }

    /// Register a handler for every custom ID with the given prefix.
    pub fn register_prefix<F, Fut>(
        &mut self,
        prefix: impl Into<String>,
        handler: F,
    ) -> Result<(), InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.register(ButtonMatcher::Prefix(prefix.into()), handler)
    }

    fn register<F, Fut>(
        &mut self,
        matcher: ButtonMatcher,
        handler: F,
    ) -> Result<(), InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        let value = match &matcher {
            ButtonMatcher::Exact(value) | ButtonMatcher::Prefix(value) => value,
        };

        if value.is_empty() {
            return Err(InteractionRegistrationError::EmptyCustomId);
        }

        if value.chars().count() > 100 {
            return Err(InteractionRegistrationError::CustomIdTooLong);
        }

        let handler: ButtonHandler = Arc::new(move |context| Box::pin(handler(context)));
        self.entries.push(RegisteredButton { matcher, handler });
        Ok(())
    }

    /// Clone all handlers matching a custom ID, preserving registration order.
    pub fn handlers_for(&self, custom_id: &str) -> Vec<ButtonHandler> {
        self.entries
            .iter()
            .filter(|entry| match &entry.matcher {
                ButtonMatcher::Exact(expected) => expected == custom_id,
                ButtonMatcher::Prefix(prefix) => custom_id.starts_with(prefix),
            })
            .map(|entry| entry.handler.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_custom_ids() {
        let mut registry = ButtonRegistry::default();

        assert_eq!(
            registry.register_exact("", |_context| async { Ok(()) }),
            Err(InteractionRegistrationError::EmptyCustomId)
        );
        assert_eq!(
            registry.register_prefix("x".repeat(101), |_context| async { Ok(()) }),
            Err(InteractionRegistrationError::CustomIdTooLong)
        );
    }

    #[test]
    fn matches_exact_and_prefix_handlers_in_registration_order() {
        let mut registry = ButtonRegistry::default();

        registry
            .register_prefix("vote:", |_context| async { Ok(()) })
            .unwrap();
        registry
            .register_exact("vote:yes", |_context| async { Ok(()) })
            .unwrap();
        registry
            .register_exact("other", |_context| async { Ok(()) })
            .unwrap();

        assert_eq!(registry.handlers_for("vote:yes").len(), 2);
        assert_eq!(registry.handlers_for("vote:no").len(), 1);
        assert!(registry.handlers_for("unknown").is_empty());
    }
}
