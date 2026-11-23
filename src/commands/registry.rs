use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::commands::CommandContext;
use crate::models::{CommandDefinition, CommandDefinitionError};

/// Error type returned by a command handler.
///
/// Handlers may return any error implementing Send + Sync + 'static.
pub type HandlerError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Result returned by an asynchronous command handler.
pub type HandlerResult = std::result::Result<(), HandlerError>;

type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send + 'static>>;
type Handler = Arc<dyn Fn(CommandContext) -> HandlerFuture + Send + Sync + 'static>;
type CheckFuture = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;
type Check = Arc<dyn Fn(CommandContext) -> CheckFuture + Send + Sync + 'static>;

struct RegisteredCommand {
    definition: CommandDefinition,
    handler: Handler,
    check: Option<Check>,
}

/// Local registry of prefix commands and their asynchronous handlers.
#[derive(Default)]
pub(crate) struct CommandRegistry {
    entries: BTreeMap<String, RegisteredCommand>,
}

impl std::fmt::Debug for CommandRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandRegistry")
            .field("commands", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl CommandRegistry {
    pub fn register<F, Fut>(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> Result<(), CommandDefinitionError>
    where
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.register_inner(
            name,
            description,
            None::<fn(CommandContext) -> std::future::Ready<bool>>,
            handler,
        )
    }

    pub fn register_with_check<C, CFut, F, Fut>(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        check: C,
        handler: F,
    ) -> Result<(), CommandDefinitionError>
    where
        C: Fn(CommandContext) -> CFut + Send + Sync + 'static,
        CFut: Future<Output = bool> + Send + 'static,
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.register_inner(name, description, Some(check), handler)
    }

    fn register_inner<C, CFut, F, Fut>(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        check: Option<C>,
        handler: F,
    ) -> Result<(), CommandDefinitionError>
    where
        C: Fn(CommandContext) -> CFut + Send + Sync + 'static,
        CFut: Future<Output = bool> + Send + 'static,
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        let definition = CommandDefinition::new(name, description)?;
        let name = definition.name().to_owned();

        if self.entries.contains_key(&name) {
            return Err(CommandDefinitionError::DuplicateName);
        }

        let handler: Handler = Arc::new(move |context| Box::pin(handler(context)));
        let check = check.map(|check| -> Check {
            Arc::new(move |context| Box::pin(check(context)))
        });

        self.entries.insert(
            name,
            RegisteredCommand {
                definition,
                handler,
                check,
            },
        );

        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn definitions(&self) -> Vec<CommandDefinition> {
        self.entries
            .values()
            .map(|entry| entry.definition.clone())
            .collect()
    }

    pub async fn dispatch(&self, name: &str, context: CommandContext) -> Option<HandlerResult> {
        let entry = self.entries.get(name)?;
        if let Some(check) = &entry.check {
            if !check(context.clone()).await {
                return Some(Ok(()));
            }
        }
        Some((entry.handler.clone())(context).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_definitions_in_deterministic_name_order() {
        let mut registry = CommandRegistry::default();

        registry
            .register("zeta", "Last command", |_context| async { Ok(()) })
            .unwrap();
        registry
            .register("alpha", "First command", |_context| async { Ok(()) })
            .unwrap();

        let names = registry
            .definitions()
            .iter()
            .map(|definition| definition.name().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(names, ["alpha", "zeta"]);
    }

    #[test]
    fn rejects_duplicate_handler_names() {
        let mut registry = CommandRegistry::default();

        registry
            .register("ping", "Ping", |_context| async { Ok(()) })
            .unwrap();

        let result = registry.register("ping", "Duplicate", |_context| async { Ok(()) });

        assert_eq!(result, Err(CommandDefinitionError::DuplicateName));
    }
    #[tokio::test]
    async fn rejected_command_check_skips_handler() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let config = crate::config::Config::builder()
            .token("yb_test-token")
            .build()
            .unwrap();
        let api = crate::api::ApiClient::new(config).unwrap();
        let message_id = crate::ids::MessageId::new(
            "123e4567-e89b-12d3-a456-426614174010",
        )
        .unwrap();
        let channel_id = crate::ids::ChannelId::new(
            "123e4567-e89b-12d3-a456-426614174011",
        )
        .unwrap();
        let context = CommandContext::new(
            api,
            "restricted".to_owned(),
            "!restricted".to_owned(),
            String::new(),
            message_id,
            channel_id,
            None,
        );
        let invoked = Arc::new(AtomicBool::new(false));
        let invoked_handler = invoked.clone();
        let mut registry = CommandRegistry::default();
        registry
            .register_with_check(
                "restricted",
                "Requires a check",
                |_context| async { false },
                move |_context| {
                    let invoked = invoked_handler.clone();
                    async move {
                        invoked.store(true, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .unwrap();

        let result = registry.dispatch("restricted", context).await.unwrap();
        assert!(result.is_ok());
        assert!(!invoked.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn accepted_command_check_runs_handler() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let config = crate::config::Config::builder()
            .token("yb_test-token")
            .build()
            .unwrap();
        let api = crate::api::ApiClient::new(config).unwrap();
        let message_id = crate::ids::MessageId::new(
            "123e4567-e89b-12d3-a456-426614174010",
        )
        .unwrap();
        let channel_id = crate::ids::ChannelId::new(
            "123e4567-e89b-12d3-a456-426614174011",
        )
        .unwrap();
        let context = CommandContext::new(
            api,
            "allowed".to_owned(),
            "!allowed".to_owned(),
            String::new(),
            message_id,
            channel_id,
            None,
        );
        let invoked = Arc::new(AtomicBool::new(false));
        let invoked_handler = invoked.clone();
        let mut registry = CommandRegistry::default();
        registry
            .register_with_check(
                "allowed",
                "Passes a check",
                |_context| async { true },
                move |_context| {
                    let invoked = invoked_handler.clone();
                    async move {
                        invoked.store(true, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .unwrap();

        let result = registry.dispatch("allowed", context).await.unwrap();
        assert!(result.is_ok());
        assert!(invoked.load(Ordering::SeqCst));
    }
}
