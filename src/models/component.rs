use std::collections::HashSet;

use serde::Serialize;
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
