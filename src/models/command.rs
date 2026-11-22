use serde::Serialize;
use thiserror::Error;

/// A command definition that can be registered with a Yapper bot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandDefinition {
    name: String,
    description: String,
}

impl CommandDefinition {
    /// Create a command definition after validating the public API limits.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandDefinitionError> {
        let name = name.into();
        let description = description.into();

        if name.is_empty()
            || name.len() > 32
            || !name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return Err(CommandDefinitionError::InvalidName);
        }

        if description.is_empty() || description.chars().count() > 100 {
            return Err(CommandDefinitionError::InvalidDescription);
        }

        Ok(Self { name, description })
    }

    /// Command name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Command description.
    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Local validation error for command definitions and command lists.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandDefinitionError {
    /// Name must be 1-32 lowercase letters, numbers, hyphens or underscores.
    #[error("command names must be 1-32 lowercase letters, numbers, hyphens or underscores")]
    InvalidName,
    /// Description must be 1-100 characters.
    #[error("command descriptions must be 1-100 characters")]
    InvalidDescription,
    /// A bot may register at most 50 commands.
    #[error("a bot may register at most 100 commands")]
    TooManyCommands,
    /// Command names must be unique within a bot.
    #[error("command names must be unique")]
    DuplicateName,
}

/// Validate a full command list before replacing the bot's remote registry.
pub(crate) fn validate_command_list(
    commands: &[CommandDefinition],
) -> Result<(), CommandDefinitionError> {
    if commands.len() > 50 {
        return Err(CommandDefinitionError::TooManyCommands);
    }

    for (index, command) in commands.iter().enumerate() {
        if commands[..index]
            .iter()
            .any(|earlier| earlier.name == command.name)
        {
            return Err(CommandDefinitionError::DuplicateName);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_command_names_and_descriptions() {
        assert!(CommandDefinition::new("ping", "Check latency").is_ok());
        assert!(CommandDefinition::new("hello_world-2", "A valid name").is_ok());
        assert_eq!(
            CommandDefinition::new("Ping", "Uppercase is not allowed"),
            Err(CommandDefinitionError::InvalidName)
        );
        assert_eq!(
            CommandDefinition::new("bad name", "Spaces are not allowed"),
            Err(CommandDefinitionError::InvalidName)
        );
        assert_eq!(
            CommandDefinition::new("ping", ""),
            Err(CommandDefinitionError::InvalidDescription)
        );
        assert_eq!(
            CommandDefinition::new("ping", "x".repeat(101)),
            Err(CommandDefinitionError::InvalidDescription)
        );
    }

    #[test]
    fn enforces_unique_names_and_the_command_cap() {
        let duplicate = vec![
            CommandDefinition::new("ping", "Ping").unwrap(),
            CommandDefinition::new("ping", "Another ping").unwrap(),
        ];
        assert_eq!(
            validate_command_list(&duplicate),
            Err(CommandDefinitionError::DuplicateName)
        );

        let commands = (0..51)
            .map(|index| CommandDefinition::new(format!("cmd-{index}"), "Command").unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            validate_command_list(&commands),
            Err(CommandDefinitionError::TooManyCommands)
        );
    }
}
