use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// Visual style for a message button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonStyle {
    /// Primary action.
    Primary,
    /// Neutral action.
    Secondary,
    /// Positive action.
    Success,
    /// Destructive or warning action.
    Danger,
    /// Opens an HTTPS URL.
    Link,
}

/// A message button that can be placed inside an action row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Button {
    #[serde(rename = "type")]
    component_type: &'static str,
    style: ButtonStyle,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    disabled: bool,
}

impl Button {
    /// Create a primary interactive button.
    pub fn primary(
        label: impl Into<String>,
        custom_id: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        Self::interactive(ButtonStyle::Primary, label, custom_id)
    }

    /// Create a secondary interactive button.
    pub fn secondary(
        label: impl Into<String>,
        custom_id: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        Self::interactive(ButtonStyle::Secondary, label, custom_id)
    }

    /// Create a success interactive button.
    pub fn success(
        label: impl Into<String>,
        custom_id: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        Self::interactive(ButtonStyle::Success, label, custom_id)
    }

    /// Create a danger interactive button.
    pub fn danger(
        label: impl Into<String>,
        custom_id: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        Self::interactive(ButtonStyle::Danger, label, custom_id)
    }

    /// Create a link button. Its URL must use HTTPS.
    pub fn link(
        label: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        let label = label.into();
        let url = url.into();
        validate_label(&label)?;

        if !url
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
        {
            return Err(ComponentValidationError::InvalidLinkUrl);
        }

        Ok(Self {
            component_type: "button",
            style: ButtonStyle::Link,
            label,
            custom_id: None,
            url: Some(url),
            emoji: None,
            disabled: false,
        })
    }

    fn interactive(
        style: ButtonStyle,
        label: impl Into<String>,
        custom_id: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        let label = label.into();
        let custom_id = custom_id.into();
        validate_label(&label)?;

        if custom_id.is_empty() || custom_id.chars().count() > 100 {
            return Err(ComponentValidationError::InvalidCustomId);
        }

        Ok(Self {
            component_type: "button",
            style,
            label,
            custom_id: Some(custom_id),
            url: None,
            emoji: None,
            disabled: false,
        })
    }

    /// Attach a Unicode or custom emoji string.
    ///
    /// Custom emoji may use the Yapper text form, such as <:name:id>.
    pub fn emoji(mut self, emoji: impl Into<String>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }

    /// Set whether the button is disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The button's display label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The button's custom ID, or None for link buttons.
    pub fn custom_id(&self) -> Option<&str> {
        self.custom_id.as_deref()
    }

    /// The button's style.
    pub fn style(&self) -> ButtonStyle {
        self.style
    }

    /// Re-check button constraints before serialization.
    pub fn validate(&self) -> Result<(), ComponentValidationError> {
        validate_label(&self.label)?;

        match self.style {
            ButtonStyle::Link => {
                if self.custom_id.is_some()
                    || !self.url.as_deref().is_some_and(|url| {
                        url.get(..8)
                            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
                    })
                {
                    return Err(ComponentValidationError::InvalidLinkUrl);
                }
            }
            _ => {
                let Some(custom_id) = self.custom_id.as_deref() else {
                    return Err(ComponentValidationError::InvalidCustomId);
                };
                if custom_id.is_empty() || custom_id.chars().count() > 100 || self.url.is_some() {
                    return Err(ComponentValidationError::InvalidCustomId);
                }
            }
        }

        Ok(())
    }
}

/// An action row containing one to five buttons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActionRow {
    #[serde(rename = "type")]
    component_type: &'static str,
    components: Vec<Button>,
}

impl ActionRow {
    /// Create an action row from one to five buttons.
    pub fn new<I>(buttons: I) -> Result<Self, ComponentValidationError>
    where
        I: IntoIterator<Item = Button>,
    {
        let components = buttons.into_iter().collect::<Vec<_>>();
        if components.is_empty() {
            return Err(ComponentValidationError::EmptyActionRow);
        }
        if components.len() > 5 {
            return Err(ComponentValidationError::TooManyButtonsInRow {
                actual: components.len(),
            });
        }

        for button in &components {
            button.validate()?;
        }

        Ok(Self {
            component_type: "row",
            components,
        })
    }

    /// Buttons in this row.
    pub fn buttons(&self) -> &[Button] {
        &self.components
    }
}

/// One selectable option in a dropdown menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectOption {
    label: String,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji: Option<Value>,
    #[serde(skip_serializing_if = "is_false")]
    default: bool,
}

impl SelectOption {
    /// Create an option with a visible label and submitted value.
    pub fn new(
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        let label = label.into();
        let value = value.into();
        if label.is_empty() {
            return Err(ComponentValidationError::EmptySelectOptionLabel);
        }
        if label.chars().count() > 100 {
            return Err(ComponentValidationError::SelectOptionLabelTooLong);
        }
        if value.is_empty() {
            return Err(ComponentValidationError::EmptySelectOptionValue);
        }
        if value.chars().count() > 100 {
            return Err(ComponentValidationError::SelectOptionValueTooLong);
        }
        Ok(Self {
            label,
            value,
            description: None,
            emoji: None,
            default: false,
        })
    }

    /// Add descriptive text beneath the option label.
    pub fn description(
        mut self,
        description: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        let description = description.into();
        if description.chars().count() > 100 {
            return Err(ComponentValidationError::SelectOptionDescriptionTooLong);
        }
        self.description = Some(description);
        Ok(self)
    }

    /// Attach a Unicode emoji or a custom emoji JSON object.
    pub fn emoji(mut self, emoji: impl Into<Value>) -> Self {
        self.emoji = Some(emoji.into());
        self
    }

    /// Mark this option as selected by default.
    pub fn default_selected(mut self, selected: bool) -> Self {
        self.default = selected;
        self
    }
}

/// Select dropdown menu component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectMenu {
    #[serde(rename = "type")]
    component_type: &'static str,
    custom_id: String,
    options: Vec<SelectOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_values: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_values: Option<u8>,
    #[serde(skip_serializing_if = "is_false")]
    disabled: bool,
}

impl SelectMenu {
    /// Create a select menu with a unique custom ID.
    pub fn new(custom_id: impl Into<String>) -> Result<Self, ComponentValidationError> {
        let custom_id = custom_id.into();
        if custom_id.is_empty() || custom_id.chars().count() > 100 {
            return Err(ComponentValidationError::InvalidSelectCustomId);
        }
        Ok(Self {
            component_type: "select",
            custom_id,
            options: Vec::new(),
            placeholder: None,
            min_values: None,
            max_values: None,
            disabled: false,
        })
    }

    /// Append one selectable option. A menu supports up to 25 options.
    pub fn option(mut self, option: SelectOption) -> Result<Self, ComponentValidationError> {
        if self.options.len() >= 25 {
            return Err(ComponentValidationError::TooManySelectOptions {
                actual: self.options.len() + 1,
            });
        }
        if self.options.iter().any(|existing| existing.value == option.value) {
            return Err(ComponentValidationError::DuplicateSelectOptionValue);
        }
        self.options.push(option);
        Ok(self)
    }

    /// Set placeholder text shown while no value is selected.
    pub fn placeholder(
        mut self,
        placeholder: impl Into<String>,
    ) -> Result<Self, ComponentValidationError> {
        let placeholder = placeholder.into();
        if placeholder.chars().count() > 150 {
            return Err(ComponentValidationError::SelectPlaceholderTooLong);
        }
        self.placeholder = Some(placeholder);
        Ok(self)
    }

    /// Set the minimum number of values that must be selected (0–25).
    pub fn min_values(mut self, minimum: usize) -> Result<Self, ComponentValidationError> {
        if minimum > 25 {
            return Err(ComponentValidationError::InvalidSelectValueRange);
        }
        self.min_values = Some(minimum as u8);
        Ok(self)
    }

    /// Set the maximum number of values that may be selected (1–25).
    pub fn max_values(mut self, maximum: usize) -> Result<Self, ComponentValidationError> {
        if !(1..=25).contains(&maximum) {
            return Err(ComponentValidationError::InvalidSelectValueRange);
        }
        self.max_values = Some(maximum as u8);
        Ok(self)
    }

    /// Set whether the select menu is disabled.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The select menu's custom ID.
    pub fn custom_id(&self) -> &str {
        &self.custom_id
    }

    /// Re-check select menu constraints before serialization.
    pub fn validate(&self) -> Result<(), ComponentValidationError> {
        if self.custom_id.is_empty() || self.custom_id.chars().count() > 100 {
            return Err(ComponentValidationError::InvalidSelectCustomId);
        }
        if self.options.is_empty() {
            return Err(ComponentValidationError::EmptySelectOptions);
        }
        if self.options.len() > 25 {
            return Err(ComponentValidationError::TooManySelectOptions {
                actual: self.options.len(),
            });
        }
        let minimum = self.min_values.unwrap_or(1);
        let maximum = self.max_values.unwrap_or(1);
        if minimum > maximum {
            return Err(ComponentValidationError::InvalidSelectValueRange);
        }
        let mut values = HashSet::new();
        for option in &self.options {
            if option.label.is_empty() {
                return Err(ComponentValidationError::EmptySelectOptionLabel);
            }
            if option.label.chars().count() > 100 {
                return Err(ComponentValidationError::SelectOptionLabelTooLong);
            }
            if option.value.is_empty() {
                return Err(ComponentValidationError::EmptySelectOptionValue);
            }
            if option.value.chars().count() > 100 {
                return Err(ComponentValidationError::SelectOptionValueTooLong);
            }
            if !values.insert(option.value.as_str()) {
                return Err(ComponentValidationError::DuplicateSelectOptionValue);
            }
        }
        Ok(())
    }
}

/// An action row containing exactly one select menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectActionRow {
    #[serde(rename = "type")]
    component_type: &'static str,
    components: Vec<SelectMenu>,
}

impl SelectActionRow {
    /// Create a row containing one validated select menu.
    pub fn new(menu: SelectMenu) -> Result<Self, ComponentValidationError> {
        menu.validate()?;
        Ok(Self {
            component_type: "row",
            components: vec![menu],
        })
    }

    /// The select menu contained in this row.
    pub fn menu(&self) -> &SelectMenu {
        &self.components[0]
    }
}

/// A message component row, containing either buttons or one select menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum MessageComponentRow {
    /// A row of one to five buttons.
    Buttons(ActionRow),
    /// A row containing one select menu.
    Select(SelectActionRow),
}

impl From<ActionRow> for MessageComponentRow {
    fn from(value: ActionRow) -> Self {
        Self::Buttons(value)
    }
}

impl From<SelectActionRow> for MessageComponentRow {
    fn from(value: SelectActionRow) -> Self {
        Self::Select(value)
    }
}

/// Validate mixed button/select rows and message-wide custom-ID uniqueness.
pub(crate) fn validate_component_rows(
    rows: &[MessageComponentRow],
) -> Result<(), ComponentValidationError> {
    if rows.len() > 5 {
        return Err(ComponentValidationError::TooManyActionRows { actual: rows.len() });
    }

    let mut custom_ids = HashSet::new();
    for row in rows {
        match row {
            MessageComponentRow::Buttons(row) => {
                if row.components.is_empty() {
                    return Err(ComponentValidationError::EmptyActionRow);
                }
                if row.components.len() > 5 {
                    return Err(ComponentValidationError::TooManyButtonsInRow {
                        actual: row.components.len(),
                    });
                }
                for button in &row.components {
                    button.validate()?;
                    if let Some(custom_id) = button.custom_id() {
                        if !custom_ids.insert(custom_id.to_owned()) {
                            return Err(ComponentValidationError::DuplicateCustomId);
                        }
                    }
                }
            }
            MessageComponentRow::Select(row) => {
                row.menu().validate()?;
                if !custom_ids.insert(row.menu().custom_id().to_owned()) {
                    return Err(ComponentValidationError::DuplicateCustomId);
                }
            }
        }
    }
    Ok(())
}

/// Validation errors for buttons and action rows.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ComponentValidationError {
    /// A button label must not be empty.
    #[error("button label must not be empty")]
    EmptyLabel,
    /// A button label is longer than Yapper's 80-character limit.
    #[error("button label must not exceed 80 characters")]
    LabelTooLong,
    /// An interactive button must have a unique custom ID from 1 to 100 characters.
    #[error("interactive button custom IDs must contain 1-100 characters")]
    InvalidCustomId,
    /// Link buttons must contain an HTTPS URL.
    #[error("link buttons must use an HTTPS URL")]
    InvalidLinkUrl,
    /// An action row must contain at least one button.
    #[error("an action row must contain at least one button")]
    EmptyActionRow,
    /// An action row may contain no more than five buttons.
    #[error("an action row may contain at most 5 buttons; received {actual}")]
    TooManyButtonsInRow {
        /// Number of buttons supplied.
        actual: usize,
    },
    /// A message may contain no more than five action rows.
    #[error("a message may contain at most 5 action rows; received {actual}")]
    TooManyActionRows {
        /// Number of rows supplied.
        actual: usize,
    },
    /// A custom ID must be unique within a message.
    #[error("button custom IDs must be unique within a message")]
    DuplicateCustomId,

    /// A select menu custom ID must contain 1–100 characters.
    #[error("select menu custom IDs must contain 1-100 characters")]
    InvalidSelectCustomId,
    /// A select menu must contain at least one option.
    #[error("a select menu must contain at least one option")]
    EmptySelectOptions,
    /// A select menu may contain no more than 25 options.
    #[error("a select menu may contain at most 25 options; received {actual}")]
    TooManySelectOptions {
        /// Number of options supplied.
        actual: usize,
    },
    /// A select option label must not be empty.
    #[error("select option labels must not be empty")]
    EmptySelectOptionLabel,
    /// A select option label may not exceed 100 characters.
    #[error("select option labels must not exceed 100 characters")]
    SelectOptionLabelTooLong,
    /// A select option value must not be empty.
    #[error("select option values must not be empty")]
    EmptySelectOptionValue,
    /// A select option value may not exceed 100 characters.
    #[error("select option values must not exceed 100 characters")]
    SelectOptionValueTooLong,
    /// A select option description may not exceed 100 characters.
    #[error("select option descriptions must not exceed 100 characters")]
    SelectOptionDescriptionTooLong,
    /// A select menu placeholder may not exceed 150 characters.
    #[error("select menu placeholders must not exceed 150 characters")]
    SelectPlaceholderTooLong,
    /// Selected-value bounds must satisfy 0 <= min <= max <= 25 and max >= 1.
    #[error("select menu min/max values must satisfy 0 <= min <= max <= 25 and max >= 1")]
    InvalidSelectValueRange,
    /// Select option values must be unique within a menu.
    #[error("select option values must be unique within a menu")]
    DuplicateSelectOptionValue,
}

fn validate_label(label: &str) -> Result<(), ComponentValidationError> {
    if label.is_empty() {
        return Err(ComponentValidationError::EmptyLabel);
    }
    if label.chars().count() > 80 {
        return Err(ComponentValidationError::LabelTooLong);
    }
    Ok(())
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Validate the row count and custom-ID uniqueness for one message.
pub(crate) fn validate_action_rows(rows: &[ActionRow]) -> Result<(), ComponentValidationError> {
    if rows.len() > 5 {
        return Err(ComponentValidationError::TooManyActionRows { actual: rows.len() });
    }

    let mut custom_ids = HashSet::new();
    for row in rows {
        if row.components.is_empty() {
            return Err(ComponentValidationError::EmptyActionRow);
        }
        if row.components.len() > 5 {
            return Err(ComponentValidationError::TooManyButtonsInRow {
                actual: row.components.len(),
            });
        }

        for button in &row.components {
            button.validate()?;
            if let Some(custom_id) = button.custom_id() {
                if !custom_ids.insert(custom_id) {
                    return Err(ComponentValidationError::DuplicateCustomId);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_button_rows_in_yapper_shape() {
        let row = ActionRow::new([
            Button::success("Yes", "vote:yes").unwrap().emoji("🔥"),
            Button::danger("No", "vote:no").unwrap().disabled(true),
            Button::link("Docs", "https://example.com").unwrap(),
        ])
        .unwrap();

        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["type"], "row");
        assert_eq!(value["components"][0]["type"], "button");
        assert_eq!(value["components"][0]["style"], "success");
        assert_eq!(value["components"][0]["custom_id"], "vote:yes");
        assert_eq!(value["components"][0]["emoji"], "🔥");
        assert_eq!(value["components"][1]["disabled"], true);
        assert_eq!(value["components"][2]["style"], "link");
        assert_eq!(value["components"][2]["url"], "https://example.com");
    }

    #[test]
    fn serializes_select_menu_and_select_action_row() {
        let menu = SelectMenu::new("food")
            .unwrap()
            .option(
                SelectOption::new("Pizza", "pizza")
                    .unwrap()
                    .description("Cheesy")
                    .unwrap()
                    .emoji("🍕")
                    .default_selected(true),
            )
            .unwrap()
            .option(SelectOption::new("Taco", "taco").unwrap())
            .unwrap()
            .placeholder("Choose a food")
            .unwrap()
            .min_values(1)
            .unwrap()
            .max_values(2)
            .unwrap();
        let row = SelectActionRow::new(menu).unwrap();
        let value = serde_json::to_value(MessageComponentRow::from(row)).unwrap();

        assert_eq!(value["type"], "row");
        assert_eq!(value["components"][0]["type"], "select");
        assert_eq!(value["components"][0]["custom_id"], "food");
        assert_eq!(value["components"][0]["options"][0]["value"], "pizza");
        assert_eq!(value["components"][0]["options"][0]["emoji"], "🍕");
        assert_eq!(value["components"][0]["min_values"], 1);
        assert_eq!(value["components"][0]["max_values"], 2);
    }

    #[test]
    fn validates_select_option_limits_and_mixed_row_custom_ids() {
        assert!(matches!(
            SelectMenu::new("x").unwrap().validate(),
            Err(ComponentValidationError::EmptySelectOptions)
        ));
        assert!(matches!(
            SelectOption::new("", "value"),
            Err(ComponentValidationError::EmptySelectOptionLabel)
        ));
        assert!(matches!(
            SelectOption::new("Option", ""),
            Err(ComponentValidationError::EmptySelectOptionValue)
        ));

        let menu = SelectMenu::new("same")
            .unwrap()
            .option(SelectOption::new("One", "one").unwrap())
            .unwrap();
        let button_row = ActionRow::new([Button::primary("One", "same").unwrap()]).unwrap();
        let select_row = SelectActionRow::new(menu).unwrap();
        assert_eq!(
            validate_component_rows(&[
                MessageComponentRow::from(button_row),
                MessageComponentRow::from(select_row),
            ]),
            Err(ComponentValidationError::DuplicateCustomId)
        );
    }

    #[test]
    fn validates_label_custom_id_and_link_url() {
        assert!(matches!(
            Button::primary("", "id"),
            Err(ComponentValidationError::EmptyLabel)
        ));
        assert!(matches!(
            Button::primary("x".repeat(81), "id"),
            Err(ComponentValidationError::LabelTooLong)
        ));
        assert!(matches!(
            Button::primary("Confirm", ""),
            Err(ComponentValidationError::InvalidCustomId)
        ));
        assert!(matches!(
            Button::link("Docs", "http://example.com"),
            Err(ComponentValidationError::InvalidLinkUrl)
        ));
    }

    #[test]
    fn validates_row_size_and_message_wide_custom_id_uniqueness() {
        let too_many = (0..6)
            .map(|index| Button::primary(format!("B{index}"), format!("id:{index}")).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            ActionRow::new(too_many),
            Err(ComponentValidationError::TooManyButtonsInRow { actual: 6 })
        ));

        let duplicate_rows = [
            ActionRow::new([Button::primary("One", "same").unwrap()]).unwrap(),
            ActionRow::new([Button::secondary("Two", "same").unwrap()]).unwrap(),
        ];
        assert_eq!(
            validate_action_rows(&duplicate_rows),
            Err(ComponentValidationError::DuplicateCustomId)
        );
    }

    #[test]
    fn enforces_message_row_limit() {
        let row = ActionRow::new([Button::primary("One", "one").unwrap()]).unwrap();
        let rows = vec![row; 6];
        assert_eq!(
            validate_action_rows(&rows),
            Err(ComponentValidationError::TooManyActionRows { actual: 6 })
        );
    }
}
