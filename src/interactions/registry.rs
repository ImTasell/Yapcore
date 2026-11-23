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

#[derive(Clone)]
pub(crate) struct RegisteredButton {
    matcher: ButtonMatcher,
    pub(crate) handler: ButtonHandler,
    pub(crate) owner: Option<String>,
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
#[derive(Default, Clone)]
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
        self.register(ButtonMatcher::Exact(custom_id.into()), None, handler)
    }

    /// Register an exact custom ID handler with optional cog ownership metadata.
    pub fn register_exact_owned<F, Fut>(
        &mut self,
        custom_id: impl Into<String>,
        owner: Option<String>,
        handler: F,
    ) -> Result<(), InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.register(ButtonMatcher::Exact(custom_id.into()), owner, handler)
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
        self.register(ButtonMatcher::Prefix(prefix.into()), None, handler)
    }

    /// Register a prefix handler with optional cog ownership metadata.
    pub fn register_prefix_owned<F, Fut>(
        &mut self,
        prefix: impl Into<String>,
        owner: Option<String>,
        handler: F,
    ) -> Result<(), InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.register(ButtonMatcher::Prefix(prefix.into()), owner, handler)
    }

    fn register<F, Fut>(
        &mut self,
        matcher: ButtonMatcher,
        owner: Option<String>,
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
        self.entries.push(RegisteredButton {
            matcher,
            handler,
            owner,
        });
        Ok(())
    }

    /// Clone all handlers matching a custom ID, preserving registration order.
    pub fn handlers_for(&self, custom_id: &str) -> Vec<RegisteredButton> {
        self.entries
            .iter()
            .filter(|entry| match &entry.matcher {
                ButtonMatcher::Exact(expected) => expected == custom_id,
                ButtonMatcher::Prefix(prefix) => custom_id.starts_with(prefix),
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_cog_ownership_for_button_handlers() {
        let mut registry = ButtonRegistry::default();
        registry
            .register_prefix_owned("vote:", Some("polls".to_owned()), |_context| async {
                Ok(())
            })
            .unwrap();
        let handlers = registry.handlers_for("vote:yes");
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].owner.as_deref(), Some("polls"));
    }

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
