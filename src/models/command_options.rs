use std::collections::BTreeMap;

use serde_json::Value;
use thiserror::Error;

/// Supported local command argument types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOptionType {
    String,
    Integer,
    Number,
    Boolean,
    User,
    Channel,
    Role,
}

/// A typed argument declaration for a prefix or slash-command handler.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandOption {
    name: String,
    description: String,
    kind: CommandOptionType,
    required: bool,
    choices: Vec<CommandOptionChoice>,
}

/// A named, typed value accepted by a command option.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandOptionChoice {
    name: String,
    value: Value,
}

impl CommandOptionChoice {
    /// Create a string choice.
    pub fn string(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: Value::String(value.into()),
        }
    }

    /// Create an integer choice.
    pub fn integer(name: impl Into<String>, value: i64) -> Self {
        Self {
            name: name.into(),
            value: Value::from(value),
        }
    }

    /// Create a numeric choice.
    pub fn number(name: impl Into<String>, value: f64) -> Result<Self, CommandOptionError> {
        let value = serde_json::Number::from_f64(value)
            .ok_or(CommandOptionError::NonFiniteNumber)?;
        Ok(Self {
            name: name.into(),
            value: Value::Number(value),
        })
    }

    /// Display name shown to users.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Validated option schema builder.
impl CommandOption {
    /// Declare a string argument.
    pub fn string(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandOptionError> {
        Self::new(name, description, CommandOptionType::String)
    }

    /// Declare an integer argument.
    pub fn integer(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandOptionError> {
        Self::new(name, description, CommandOptionType::Integer)
    }

    /// Declare a floating-point argument.
    pub fn number(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandOptionError> {
        Self::new(name, description, CommandOptionType::Number)
    }

    /// Declare a true/false argument.
    pub fn boolean(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandOptionError> {
        Self::new(name, description, CommandOptionType::Boolean)
    }

    /// Declare a user ID argument.
    pub fn user(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandOptionError> {
        Self::new(name, description, CommandOptionType::User)
    }

    /// Declare a channel ID argument.
    pub fn channel(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandOptionError> {
        Self::new(name, description, CommandOptionType::Channel)
    }

    /// Declare a role ID argument.
    pub fn role(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, CommandOptionError> {
        Self::new(name, description, CommandOptionType::Role)
    }

    fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        kind: CommandOptionType,
    ) -> Result<Self, CommandOptionError> {
        let name = name.into();
        let description = description.into();
        if name.is_empty()
            || name.len() > 32
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
        {
            return Err(CommandOptionError::InvalidName);
        }
        if description.is_empty() || description.chars().count() > 100 {
            return Err(CommandOptionError::InvalidDescription);
        }
        Ok(Self {
            name,
            description,
            kind,
            required: false,
            choices: Vec::new(),
        })
    }

    /// Mark this argument as required.
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Add an allowed choice. Choice values must match this option's type.
    pub fn choice(mut self, choice: CommandOptionChoice) -> Result<Self, CommandOptionError> {
        if !choice_matches(self.kind, &choice.value) {
            return Err(CommandOptionError::ChoiceTypeMismatch);
        }
        if self.choices.len() >= 25 {
            return Err(CommandOptionError::TooManyChoices);
        }
        if self
            .choices
            .iter()
            .any(|existing| existing.value == choice.value || existing.name == choice.name)
        {
            return Err(CommandOptionError::DuplicateChoice);
        }
        self.choices.push(choice);
        Ok(self)
    }

    /// Argument name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Argument description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Argument type.
    pub fn kind(&self) -> CommandOptionType {
        self.kind
    }

    /// Whether the argument is required.
    pub fn is_required(&self) -> bool {
        self.required
    }
}

/// Parsed, validated values for a command invocation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedCommandOptions {
    values: BTreeMap<String, Value>,
}

impl ParsedCommandOptions {
    /// Validate option count, unique names and required-before-optional ordering.
    pub(crate) fn validate_schema(
        schema: &[CommandOption],
    ) -> Result<(), CommandOptionError> {
        if schema.len() > 10 {
            return Err(CommandOptionError::TooManyOptions);
        }

        let mut schema_names = std::collections::BTreeSet::new();
        let mut optional_seen = false;
        for option in schema {
            if !schema_names.insert(option.name.as_str()) {
                return Err(CommandOptionError::DuplicateOption(option.name.clone()));
            }
            if option.required && optional_seen {
                return Err(CommandOptionError::RequiredOptionAfterOptional(
                    option.name.clone(),
                ));
            }
            optional_seen |= !option.required;
        }
        Ok(())
    }

    /// Parse whitespace-separated positional values and named `name=value` pairs.
    ///
    /// Quoted values support single or double quotes. User, channel and role
    /// arguments are retained as strings because Yapper IDs are opaque.
    pub fn parse(schema: &[CommandOption], input: &str) -> Result<Self, CommandOptionError> {
        Self::validate_schema(schema)?;
        let tokens = tokenize(input)?;
        let mut positional = Vec::new();
        let mut named = BTreeMap::new();
        for token in tokens {
            if let Some((key, value)) = token.split_once('=') {
                if key.is_empty() || value.is_empty() {
                    return Err(CommandOptionError::InvalidNamedArgument(token));
                }
                if named.insert(key.to_owned(), value.to_owned()).is_some() {
                    return Err(CommandOptionError::DuplicateArgument(key.to_owned()));
                }
            } else {
                positional.push(token);
            }
        }

        let mut values = BTreeMap::new();
        let mut positional_index = 0;
        for option in schema {
            let raw = if let Some(value) = named.remove(&option.name) {
                Some(value)
            } else if positional_index < positional.len() {
                let value = positional[positional_index].clone();
                positional_index += 1;
                Some(value)
            } else {
                None
            };
            let Some(raw) = raw else {
                if option.required {
                    return Err(CommandOptionError::MissingRequired(option.name.clone()));
                }
                continue;
            };
            let value = parse_value(option.kind, &raw, &option.name)?;
            if !option.choices.is_empty()
                && !option.choices.iter().any(|choice| choice.value == value)
            {
                return Err(CommandOptionError::InvalidChoice(option.name.clone()));
            }
            values.insert(option.name.clone(), value);
        }
        if let Some((unknown, _)) = named.into_iter().next() {
            return Err(CommandOptionError::UnknownArgument(unknown));
        }
        if positional_index < positional.len() {
            return Err(CommandOptionError::TooManyArguments);
        }
        Ok(Self { values })
    }

    /// Return a string-like value (including user/channel/role IDs).
    pub fn get_string(&self, name: &str) -> Option<&str> {
        self.values.get(name)?.as_str()
    }

    /// Return an integer value.
    pub fn get_integer(&self, name: &str) -> Option<i64> {
        self.values.get(name)?.as_i64()
    }

    /// Return a numeric value.
    pub fn get_number(&self, name: &str) -> Option<f64> {
        self.values.get(name)?.as_f64()
    }

    /// Return a boolean value.
    pub fn get_boolean(&self, name: &str) -> Option<bool> {
        self.values.get(name)?.as_bool()
    }

    /// Return a validated Yapper user ID.
    pub fn get_user(&self, name: &str) -> Option<crate::ids::UserId> {
        crate::ids::UserId::new(self.get_string(name)?.to_owned()).ok()
    }

    /// Return a validated Yapper channel ID.
    pub fn get_channel(&self, name: &str) -> Option<crate::ids::ChannelId> {
        crate::ids::ChannelId::new(self.get_string(name)?.to_owned()).ok()
    }

    /// Return a validated Yapper role ID.
    pub fn get_role(&self, name: &str) -> Option<crate::ids::RoleId> {
        crate::ids::RoleId::new(self.get_string(name)?.to_owned()).ok()
    }

    /// Return the complete parsed value map.
    pub fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.values
    }
}

fn parse_value(
    kind: CommandOptionType,
    raw: &str,
    option_name: &str,
) -> Result<Value, CommandOptionError> {
    match kind {
        CommandOptionType::String => Ok(Value::String(raw.to_owned())),
        CommandOptionType::User => {
            let id = raw
                .strip_prefix("<@!")
                .or_else(|| raw.strip_prefix("<@"))
                .unwrap_or(raw);
            let id = id.strip_suffix('>').unwrap_or(id);
            crate::ids::UserId::new(id.to_owned())
                .map(|id| Value::String(id.into_string()))
                .map_err(|_| CommandOptionError::InvalidValue(option_name.to_owned()))
        }
        CommandOptionType::Channel => {
            let id = raw.strip_prefix("<#").unwrap_or(raw);
            let id = id.strip_suffix('>').unwrap_or(id);
            crate::ids::ChannelId::new(id.to_owned())
                .map(|id| Value::String(id.into_string()))
                .map_err(|_| CommandOptionError::InvalidValue(option_name.to_owned()))
        }
        CommandOptionType::Role => {
            let id = raw.strip_prefix("<@&").unwrap_or(raw);
            let id = id.strip_suffix('>').unwrap_or(id);
            crate::ids::RoleId::new(id.to_owned())
                .map(|id| Value::String(id.into_string()))
                .map_err(|_| CommandOptionError::InvalidValue(option_name.to_owned()))
        }
        CommandOptionType::Integer => raw
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| CommandOptionError::InvalidValue(option_name.to_owned())),
        CommandOptionType::Number => {
            let number = raw
                .parse::<f64>()
                .map_err(|_| CommandOptionError::InvalidValue(option_name.to_owned()))?;
            serde_json::Number::from_f64(number)
                .map(Value::Number)
                .ok_or(CommandOptionError::NonFiniteNumber)
        }
        CommandOptionType::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => Ok(Value::Bool(true)),
            "false" | "no" | "off" => Ok(Value::Bool(false)),
            _ => Err(CommandOptionError::InvalidValue(raw.to_owned())),
        },
    }
}

fn choice_matches(kind: CommandOptionType, value: &Value) -> bool {
    match kind {
        CommandOptionType::String
        | CommandOptionType::User
        | CommandOptionType::Channel
        | CommandOptionType::Role => value.is_string(),
        CommandOptionType::Integer => value.as_i64().is_some(),
        CommandOptionType::Number => value.is_number(),
        CommandOptionType::Boolean => value.is_boolean(),
    }
}

fn tokenize(input: &str) -> Result<Vec<String>, CommandOptionError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if quote.is_some() || escaped {
        return Err(CommandOptionError::UnterminatedQuote);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

impl CommandOptionError {
    /// Build a safe, non-echoing validation response for bot messages.
    pub(crate) fn user_message(&self) -> String {
        match self {
            Self::MissingRequired(name) => format!("Missing required option: {name}."),
            Self::InvalidChoice(name) => {
                format!("Option {name} must use one of its allowed choices.")
            }
            Self::InvalidValue(name) => format!("Option {name} has an invalid value."),
            Self::TooManyArguments => "Too many arguments were supplied.".to_owned(),
            Self::UnknownArgument(_) => "An unknown named option was supplied.".to_owned(),
            Self::DuplicateArgument(_) => "An option was supplied more than once.".to_owned(),
            Self::InvalidNamedArgument(_) => "A named option is malformed.".to_owned(),
            Self::UnterminatedQuote => "A quoted argument was not closed.".to_owned(),
            Self::NonFiniteNumber => "Numeric values must be finite.".to_owned(),
            _ => "The command option schema is invalid.".to_owned(),
        }
    }
}

/// Errors raised while declaring or parsing command options.
#[derive(Debug, Clone, PartialEq, Error)]
#[non_exhaustive]
pub enum CommandOptionError {
    #[error("option names must be 1-32 lowercase letters, numbers or underscores")]
    InvalidName,
    #[error("option descriptions must be 1-100 characters")]
    InvalidDescription,
    #[error("choice type does not match its option")]
    ChoiceTypeMismatch,
    #[error("an option may contain at most 25 choices")]
    TooManyChoices,
    #[error("choice names and values must be unique within an option")]
    DuplicateChoice,
    #[error("option names must be unique within a command schema")]
    DuplicateOption(String),
    #[error("a command may declare at most 10 options")]
    TooManyOptions,
    #[error("required option {0} must appear before all optional options")]
    RequiredOptionAfterOptional(String),
    #[error("required argument '{0}' is missing")]
    MissingRequired(String),
    #[error("unknown named argument '{0}'")]
    UnknownArgument(String),
    #[error("argument '{0}' was supplied more than once")]
    DuplicateArgument(String),
    #[error("named argument is malformed: '{0}'")]
    InvalidNamedArgument(String),
    #[error("too many positional arguments")]
    TooManyArguments,
    #[error("invalid value '{0}'")]
    InvalidValue(String),
    #[error("argument '{0}' is not one of its declared choices")]
    InvalidChoice(String),
    #[error("unterminated quoted argument")]
    UnterminatedQuote,
    #[error("numeric argument must be finite")]
    NonFiniteNumber,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_positional_and_named_arguments() {
        let schema = [
            CommandOption::string("name", "Display name").unwrap().required(true),
            CommandOption::integer("count", "Repeat count").unwrap(),
            CommandOption::boolean("enabled", "Enable feature").unwrap(),
            CommandOption::user("member", "Target member").unwrap(),
        ];
        let parsed = ParsedCommandOptions::parse(
            &schema,
            r#" "Altix Dev" 12 enabled=true member=123e4567-e89b-12d3-a456-426614174003 "#,
        ).unwrap();
        assert_eq!(parsed.get_string("name"), Some("Altix Dev"));
        assert_eq!(parsed.get_integer("count"), Some(12));
        assert_eq!(parsed.get_boolean("enabled"), Some(true));
        assert_eq!(
            parsed.get_user("member").unwrap().as_str(),
            "123e4567-e89b-12d3-a456-426614174003"
        );
    }

    #[test]
    fn accepts_mentioned_user_channel_and_role_ids() {
        let user_id = "123e4567-e89b-12d3-a456-426614174003";
        let channel_id = "123e4567-e89b-12d3-a456-426614174004";
        let role_id = "123e4567-e89b-12d3-a456-426614174005";
        let schema = [
            CommandOption::user("member", "Member").unwrap(),
            CommandOption::channel("channel", "Channel").unwrap(),
            CommandOption::role("role", "Role").unwrap(),
        ];
        let parsed = ParsedCommandOptions::parse(
            &schema,
            &format!("member=<@!{user_id}> channel=<#{channel_id}> role=<@&{role_id}>"),
        )
        .unwrap();
        assert_eq!(parsed.get_user("member").unwrap().as_str(), user_id);
        assert_eq!(parsed.get_channel("channel").unwrap().as_str(), channel_id);
        assert_eq!(parsed.get_role("role").unwrap().as_str(), role_id);
    }

    #[test]
    fn validates_platform_ids_and_returns_typed_id_values() {
        let schema = [CommandOption::user("member", "Member").unwrap()];
        assert!(matches!(
            ParsedCommandOptions::parse(&schema, "not-an-id"),
            Err(CommandOptionError::InvalidValue(_))
        ));
        let parsed = ParsedCommandOptions::parse(
            &schema,
            "123e4567-e89b-12d3-a456-426614174003",
        )
        .unwrap();
        assert_eq!(
            parsed.get_user("member").unwrap().as_str(),
            "123e4567-e89b-12d3-a456-426614174003"
        );
    }

    #[test]
    fn validates_required_choices_and_bad_values() {
        let schema = [CommandOption::string("mode", "Mode")
            .unwrap()
            .choice(CommandOptionChoice::string("Fast", "fast")).unwrap()
            .required(true)];
        assert!(matches!(
            ParsedCommandOptions::parse(&schema, ""),
            Err(CommandOptionError::MissingRequired(_))
        ));
        assert!(matches!(
            ParsedCommandOptions::parse(&schema, "slow"),
            Err(CommandOptionError::InvalidChoice(_))
        ));
        let integer = [CommandOption::integer("count", "Count").unwrap()];
        assert!(matches!(
            ParsedCommandOptions::parse(&integer, "nope"),
            Err(CommandOptionError::InvalidValue(_))
        ));
    }

    #[test]
    fn user_facing_errors_never_echo_untrusted_argument_text() {
        let error = CommandOptionError::InvalidNamedArgument("@everyone".to_owned());
        assert_eq!(error.user_message(), "A named option is malformed.");
        assert!(!error.user_message().contains("@everyone"));
    }

    #[test]
    fn rejects_duplicate_schema_names_and_accepts_numeric_choices() {
        let duplicate = [
            CommandOption::string("name", "Name").unwrap(),
            CommandOption::integer("name", "Second name").unwrap(),
        ];
        assert!(matches!(
            ParsedCommandOptions::parse(&duplicate, "value"),
            Err(CommandOptionError::DuplicateOption(_))
        ));

        let schema = [CommandOption::number("ratio", "Ratio")
            .unwrap()
            .choice(CommandOptionChoice::integer("Whole", 2))
            .unwrap()];
        assert_eq!(
            ParsedCommandOptions::parse(&schema, "2")
                .unwrap()
                .get_number("ratio"),
            Some(2.0)
        );
    }

    #[test]
    fn validates_option_count_and_required_order() {
        let too_many = (0..11)
            .map(|index| CommandOption::string(format!("option-{index}"), "Option").unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            ParsedCommandOptions::parse(&too_many, ""),
            Err(CommandOptionError::TooManyOptions)
        ));

        let required_after_optional = [
            CommandOption::string("optional", "Optional").unwrap(),
            CommandOption::string("required", "Required")
                .unwrap()
                .required(true),
        ];
        assert!(matches!(
            ParsedCommandOptions::parse(&required_after_optional, ""),
            Err(CommandOptionError::RequiredOptionAfterOptional(name)) if name == "required"
        ));
    }

    #[test]
    fn rejects_duplicate_unknown_and_unterminated_arguments() {
        let schema = [CommandOption::string("name", "Name").unwrap()];
        assert!(matches!(
            ParsedCommandOptions::parse(&schema, "name=a name=b"),
            Err(CommandOptionError::DuplicateArgument(_))
        ));
        assert!(matches!(
            ParsedCommandOptions::parse(&schema, "other=a"),
            Err(CommandOptionError::UnknownArgument(_))
        ));
        assert!(matches!(
            ParsedCommandOptions::parse(&schema, r#""unfinished"#),
            Err(CommandOptionError::UnterminatedQuote)
        ));
    }
}
