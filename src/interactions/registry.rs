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
    /// Only button, select and modal-submit interaction kinds are supported.
    #[error("interaction kind must be 'button', 'select' or 'modal_submit'")]
    UnsupportedKind,
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
    pub(crate) kind: String,
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
        self.register("button", ButtonMatcher::Exact(custom_id.into()), None, handler)
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
        self.register("button", ButtonMatcher::Exact(custom_id.into()), owner, handler)
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
        self.register("button", ButtonMatcher::Prefix(prefix.into()), None, handler)
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
        self.register("button", ButtonMatcher::Prefix(prefix.into()), owner, handler)
    }

    /// Register an exact custom ID for a select or modal-submit interaction.
    pub fn register_kind_owned<F, Fut>(
        &mut self,
        kind: impl Into<String>,
        custom_id: impl Into<String>,
        owner: Option<String>,
        handler: F,
    ) -> Result<(), InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        let kind = kind.into();
        if !matches!(kind.as_str(), "button" | "select" | "modal_submit") {
            return Err(InteractionRegistrationError::UnsupportedKind);
        }
        self.register(&kind, ButtonMatcher::Exact(custom_id.into()), owner, handler)
    }

    fn register<F, Fut>(
        &mut self,
        kind: &str,
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
            kind: kind.to_owned(),
            handler,
            owner,
        });
        Ok(())
    }

    /// Remove every button handler registered by one cog and return the number removed.
    pub fn remove_owner(&mut self, owner: &str) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.owner.as_deref() != Some(owner));
        before - self.entries.len()
    }

    /// Clone all handlers matching a custom ID, preserving registration order.
    pub fn handlers_for(&self, kind: &str, custom_id: &str) -> Vec<RegisteredButton> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == kind && match &entry.matcher {
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
    fn removes_only_button_handlers_owned_by_selected_cog() {
        let mut registry = ButtonRegistry::default();
        registry
            .register_prefix_owned("alpha:", Some("alpha".to_owned()), |_context| async {
                Ok(())
            })
            .unwrap();
        registry
            .register_exact("alpha:other", |_context| async { Ok(()) })
            .unwrap();

        assert_eq!(registry.remove_owner("alpha"), 1);
        assert_eq!(registry.handlers_for("button", "alpha:other").len(), 1);
    }

    #[test]
    fn preserves_cog_ownership_for_button_handlers() {
        let mut registry = ButtonRegistry::default();
        registry
            .register_prefix_owned("vote:", Some("polls".to_owned()), |_context| async {
                Ok(())
            })
            .unwrap();
        let handlers = registry.handlers_for("button", "vote:yes");
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

        assert_eq!(registry.handlers_for("button", "vote:yes").len(), 2);
        assert_eq!(registry.handlers_for("button", "vote:no").len(), 1);
        assert!(registry.handlers_for("button", "unknown").is_empty());
    }
}
