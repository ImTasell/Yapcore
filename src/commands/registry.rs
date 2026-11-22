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

struct RegisteredCommand {
    definition: CommandDefinition,
    handler: Handler,
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
        let definition = CommandDefinition::new(name, description)?;
        let name = definition.name().to_owned();

        if self.entries.contains_key(&name) {
            return Err(CommandDefinitionError::DuplicateName);
        }

        let handler: Handler = Arc::new(move |context| Box::pin(handler(context)));

        self.entries.insert(
            name,
            RegisteredCommand {
                definition,
                handler,
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
        let handler = self.entries.get(name)?.handler.clone();
        Some(handler(context).await)
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
}
