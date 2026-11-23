use serde::Serialize;
use thiserror::Error;

/// Supported initial interaction callback types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractionResponseType {
    /// Send a new message in the channel.
    Reply,
    /// Send a message visible only to the interaction user.
    Ephemeral,
    /// Update the message the interaction originated from.
    Update,
    /// Acknowledge without sending a visible message.
    Ack,
    /// Open a modal as the initial interaction response.
    Modal,
}

/// Text-input presentation style for a modal field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextInputStyle {
    /// Single-line text field.
    Short,
    /// Multi-line paragraph field.
    Paragraph,
}

/// A text field that can be placed inside a modal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextInput {
    #[serde(rename = "type")]
    component_type: &'static str,
    custom_id: String,
    label: String,
    style: TextInputStyle,
    #[serde(skip_serializing_if = "Option::is_none")]
    required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl TextInput {
    /// Create a single-line text input.
    pub fn short(
        custom_id: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<Self, ModalValidationError> {
        Self::new(custom_id, label, TextInputStyle::Short)
    }

    /// Create a multi-line paragraph input.
    pub fn paragraph(
        custom_id: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<Self, ModalValidationError> {
        Self::new(custom_id, label, TextInputStyle::Paragraph)
    }

    fn new(
        custom_id: impl Into<String>,
        label: impl Into<String>,
        style: TextInputStyle,
    ) -> Result<Self, ModalValidationError> {
        let custom_id = custom_id.into();
        if custom_id.is_empty() || custom_id.chars().count() > 100 {
            return Err(ModalValidationError::InvalidCustomId);
        }
        let label = label.into();
        if label.is_empty() {
            return Err(ModalValidationError::EmptyLabel);
        }

        Ok(Self {
            component_type: "text_input",
            custom_id,
            label,
            style,
            required: None,
            min_length: None,
            max_length: None,
            value: None,
            placeholder: None,
            description: None,
        })
    }

    /// Set whether this field must be filled in.
    pub fn required(mut self, required: bool) -> Self {
        self.required = Some(required);
        self
    }

    /// Set inclusive minimum and maximum character lengths.
    pub fn length(
        mut self,
        min: Option<usize>,
        max: Option<usize>,
    ) -> Result<Self, ModalValidationError> {
        if min.is_some_and(|value| value > 4000)
            || max.is_some_and(|value| value > 4000)
            || matches!((min, max), (Some(min), Some(max)) if min > max)
        {
            return Err(ModalValidationError::InvalidLength);
        }
        self.min_length = min;
        self.max_length = max;
        Ok(self)
    }

    /// Prefill the input with a value.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Set placeholder text.
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set the smaller descriptive text shown under the label.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// A modal form that can be opened as an interaction's first response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Modal {
    custom_id: String,
    title: String,
    components: Vec<TextInput>,
}

impl Modal {
    /// Create a modal from up to five text inputs.
    pub fn new<I>(
        custom_id: impl Into<String>,
        title: impl Into<String>,
        inputs: I,
    ) -> Result<Self, ModalValidationError>
    where
        I: IntoIterator<Item = TextInput>,
    {
        let custom_id = custom_id.into();
        if custom_id.is_empty() || custom_id.chars().count() > 100 {
            return Err(ModalValidationError::InvalidCustomId);
        }
        let title = title.into();
        if title.is_empty() {
            return Err(ModalValidationError::EmptyTitle);
        }
        let components = inputs.into_iter().collect::<Vec<_>>();
        if components.len() > 5 {
            return Err(ModalValidationError::TooManyInputs {
                actual: components.len(),
            });
        }
        Ok(Self {
            custom_id,
            title,
            components,
        })
    }
}

/// Validation errors for modal and text-input builders.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModalValidationError {
    /// A modal or input custom ID must contain 1–100 characters.
    #[error("modal and input custom IDs must contain 1-100 characters")]
    InvalidCustomId,
    /// A text-input label must not be empty.
    #[error("text-input label must not be empty")]
    EmptyLabel,
    /// A modal title must not be empty.
    #[error("modal title must not be empty")]
    EmptyTitle,
    /// A modal may contain no more than five inputs.
    #[error("a modal may contain at most 5 inputs; received {actual}")]
    TooManyInputs {
        /// Number of inputs supplied.
        actual: usize,
    },
    /// Text-input lengths must be at most 4000 and min must not exceed max.
    #[error("text-input lengths must be at most 4000 and minimum must not exceed maximum")]
    InvalidLength,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_modal_and_text_inputs_in_upstream_shape() {
        let modal = Modal::new(
            "application",
            "Apply",
            [
                TextInput::short("name", "Your name")
                    .unwrap()
                    .required(true)
                    .placeholder("Name"),
                TextInput::paragraph("why", "Why?")
                    .unwrap()
                    .length(Some(1), Some(4000))
                    .unwrap(),
            ],
        )
        .unwrap();

        let value = serde_json::to_value(modal).unwrap();
        assert_eq!(value["custom_id"], "application");
        assert_eq!(value["title"], "Apply");
        assert_eq!(value["components"][0]["type"], "text_input");
        assert_eq!(value["components"][0]["style"], "short");
        assert_eq!(value["components"][0]["required"].as_bool(), Some(true));
        assert_eq!(value["components"][1]["style"], "paragraph");
        assert_eq!(value["components"][1]["max_length"], 4000);
    }

    #[test]
    fn rejects_invalid_modal_and_input_limits() {
        assert!(matches!(
            TextInput::short("", "Name"),
            Err(ModalValidationError::InvalidCustomId)
        ));
        assert!(matches!(
            TextInput::short("name", ""),
            Err(ModalValidationError::EmptyLabel)
        ));
        assert!(matches!(
            TextInput::short("name", "Name")
                .unwrap()
                .length(Some(10), Some(5)),
            Err(ModalValidationError::InvalidLength)
        ));

        let inputs = (0..6)
            .map(|index| TextInput::short(format!("field-{index}"), "Field").unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            Modal::new("form", "Form", inputs),
            Err(ModalValidationError::TooManyInputs { actual: 6 })
        ));
    }
}
