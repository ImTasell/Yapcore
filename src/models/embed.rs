use serde::Serialize;
use thiserror::Error;

/// An embed field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbedField {
    name: String,
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline: Option<bool>,
}

impl EmbedField {
    /// Create a field after validating its name and value lengths.
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, EmbedValidationError> {
        let name = name.into();
        let value = value.into();

        validate_length("field name", &name, 256)?;
        validate_length("field value", &value, 1_024)?;

        if name.is_empty() || value.is_empty() {
            return Err(EmbedValidationError::EmptyField);
        }

        Ok(Self {
            name,
            value,
            inline: None,
        })
    }

    /// Set whether this field should be displayed inline.
    pub fn inline(mut self, inline: bool) -> Self {
        self.inline = Some(inline);
        self
    }

    /// Field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Field value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether the field is inline.
    pub fn is_inline(&self) -> Option<bool> {
        self.inline
    }
}

/// Author metadata displayed at the top of an embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbedAuthor {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
}

impl EmbedAuthor {
    /// Create author metadata.
    pub fn new(name: impl Into<String>) -> Result<Self, EmbedValidationError> {
        let name = name.into();
        validate_length("author name", &name, 256)?;
        if name.is_empty() {
            return Err(EmbedValidationError::EmptyAuthor);
        }

        Ok(Self {
            name,
            url: None,
            icon_url: None,
        })
    }

    /// Set the author's link URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the author's icon URL.
    pub fn icon_url(mut self, url: impl Into<String>) -> Self {
        self.icon_url = Some(url.into());
        self
    }
}

/// Footer metadata displayed at the bottom of an embed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbedFooter {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
}

impl EmbedFooter {
    /// Create footer metadata.
    pub fn new(text: impl Into<String>) -> Result<Self, EmbedValidationError> {
        let text = text.into();
        validate_length("footer text", &text, 2_048)?;
        if text.is_empty() {
            return Err(EmbedValidationError::EmptyFooter);
        }

        Ok(Self {
            text,
            icon_url: None,
        })
    }

    /// Set the footer icon URL.
    pub fn icon_url(mut self, url: impl Into<String>) -> Self {
        self.icon_url = Some(url.into());
        self
    }
}

/// An embed attached to a message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Embed {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<EmbedAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    footer: Option<EmbedFooter>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fields: Vec<EmbedField>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
}

impl Embed {
    /// Start building an embed.
    pub fn builder() -> EmbedBuilder {
        EmbedBuilder::default()
    }

    /// Title text.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Description text.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Embed fields.
    pub fn fields(&self) -> &[EmbedField] {
        &self.fields
    }

    /// Total text characters counted against the 6,000-character aggregate limit.
    pub fn text_characters(&self) -> usize {
        let mut count = self.title.as_ref().map_or(0, |value| value.chars().count());
        count += self
            .description
            .as_ref()
            .map_or(0, |value| value.chars().count());

        if let Some(author) = &self.author {
            count += author.name.chars().count();
        }
        if let Some(footer) = &self.footer {
            count += footer.text.chars().count();
        }

        for field in &self.fields {
            count += field.name.chars().count();
            count += field.value.chars().count();
        }

        count
    }

    /// Validate individual and aggregate embed limits.
    pub fn validate(&self) -> Result<(), EmbedValidationError> {
        if let Some(title) = &self.title {
            validate_length("title", title, 256)?;
        }
        if let Some(description) = &self.description {
            validate_length("description", description, 4_096)?;
        }
        if let Some(author) = &self.author {
            validate_length("author name", &author.name, 256)?;
        }
        if let Some(footer) = &self.footer {
            validate_length("footer text", &footer.text, 2_048)?;
        }
        if self.fields.len() > 10 {
            return Err(EmbedValidationError::TooManyFields {
                actual: self.fields.len(),
            });
        }
        for field in &self.fields {
            validate_length("field name", &field.name, 256)?;
            validate_length("field value", &field.value, 1_024)?;
        }
        if let Some(url) = &self.url {
            validate_length("embed URL", url, 512)?;
            let is_http = url
                .get(..7)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"));
            let is_https = url
                .get(..8)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"));
            if !is_http && !is_https {
                return Err(EmbedValidationError::InvalidLinkUrl);
            }
        }

        for image_url in [&self.image, &self.thumbnail].into_iter().flatten() {
            validate_length("embed image URL", image_url, 512)?;
            if !image_url
                .get(..8)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
            {
                return Err(EmbedValidationError::InvalidImageUrl);
            }
        }

        if let Some(color) = self.color {
            if color > 0xFF_FFFF {
                return Err(EmbedValidationError::InvalidColor { actual: color });
            }
        }

        let total = self.text_characters();
        if total > 6_000 {
            return Err(EmbedValidationError::AggregateTextTooLong { actual: total });
        }

        Ok(())
    }
}

/// Builder for an embed.
#[derive(Debug, Clone, Default)]
pub struct EmbedBuilder {
    embed: Embed,
}

impl EmbedBuilder {
    /// Set the title (maximum 256 characters).
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.embed.title = Some(title.into());
        self
    }

    /// Set the description (maximum 4,096 characters).
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.embed.description = Some(description.into());
        self
    }

    /// Set the link URL (maximum 512 characters; HTTP and HTTPS are accepted by the API).
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.embed.url = Some(url.into());
        self
    }

    /// Set the 24-bit RGB color.
    pub fn color(mut self, color: u32) -> Self {
        self.embed.color = Some(color);
        self
    }

    /// Set author metadata.
    pub fn author(mut self, author: EmbedAuthor) -> Self {
        self.embed.author = Some(author);
        self
    }

    /// Set footer metadata.
    pub fn footer(mut self, footer: EmbedFooter) -> Self {
        self.embed.footer = Some(footer);
        self
    }

    /// Append one field (up to ten fields per embed).
    pub fn field(mut self, field: EmbedField) -> Self {
        self.embed.fields.push(field);
        self
    }

    /// Set an HTTPS image URL. The server still enforces its image-host allowlist.
    pub fn image_url(mut self, url: impl Into<String>) -> Self {
        self.embed.image = Some(url.into());
        self
    }

    /// Set an HTTPS thumbnail URL. The server still enforces its image-host allowlist.
    pub fn thumbnail_url(mut self, url: impl Into<String>) -> Self {
        self.embed.thumbnail = Some(url.into());
        self
    }

    /// Set an ISO-8601 timestamp.
    pub fn timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.embed.timestamp = Some(timestamp.into());
        self
    }

    /// Validate and finish the embed.
    pub fn build(self) -> Result<Embed, EmbedValidationError> {
        self.embed.validate()?;
        Ok(self.embed)
    }
}

/// Local validation failure for embeds and embed fields.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum EmbedValidationError {
    /// A named embed component exceeded its documented character limit.
    #[error("{component} is {actual} characters; the maximum is {max}")]
    TextTooLong {
        /// Component name.
        component: &'static str,
        /// Actual character count.
        actual: usize,
        /// Maximum allowed character count.
        max: usize,
    },
    /// An embed field must have a non-empty name and value.
    #[error("embed fields must have a non-empty name and value")]
    EmptyField,
    /// An author must have a non-empty name.
    #[error("embed author name must not be empty")]
    EmptyAuthor,
    /// A footer must have non-empty text.
    #[error("embed footer text must not be empty")]
    EmptyFooter,
    /// An embed may contain no more than ten fields.
    #[error("an embed may contain at most 10 fields; received {actual}")]
    TooManyFields {
        /// Number of supplied fields.
        actual: usize,
    },
    /// Combined embed text exceeded 6,000 characters.
    #[error("combined embed text is {actual} characters; the maximum is 6000")]
    AggregateTextTooLong {
        /// Actual combined character count.
        actual: usize,
    },
    /// Color must fit into 24-bit RGB.
    #[error("embed color must be 24-bit RGB; received {actual:#x}")]
    InvalidColor {
        /// Supplied color.
        actual: u32,
    },

    /// Embed link URLs must use HTTP or HTTPS.
    #[error("embed link URL must use HTTP or HTTPS")]
    InvalidLinkUrl,

    /// Embed image URLs must use HTTPS.
    #[error("embed image and thumbnail URLs must use HTTPS")]
    InvalidImageUrl,
}

fn validate_length(
    component: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), EmbedValidationError> {
    let actual = value.chars().count();
    if actual > maximum {
        return Err(EmbedValidationError::TextTooLong {
            component,
            actual,
            max: maximum,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_serializes_an_embed_without_unset_fields() {
        let embed = Embed::builder()
            .title("Yapcore")
            .description("Rust bot framework")
            .color(0x0058_65FF)
            .field(EmbedField::new("Language", "Rust").unwrap().inline(true))
            .footer(EmbedFooter::new("Built with Yapcore").unwrap())
            .build()
            .unwrap();

        let value = serde_json::to_value(&embed).unwrap();
        assert_eq!(value["title"], "Yapcore");
        assert_eq!(value["fields"][0]["name"], "Language");
        assert_eq!(value["fields"][0]["inline"], true);
        assert!(value.get("image").is_none());
        assert!(value.get("thumbnail").is_none());
    }

    #[test]
    fn rejects_field_and_aggregate_limit_violations() {
        assert!(matches!(
            EmbedField::new("x".repeat(257), "value"),
            Err(EmbedValidationError::TextTooLong {
                component: "field name",
                ..
            })
        ));

        let embed = (0..10).fold(Embed::builder(), |builder, index| {
            builder.field(EmbedField::new(format!("field-{index}"), "x".repeat(1_024)).unwrap())
        });

        assert!(matches!(
            embed.build(),
            Err(EmbedValidationError::AggregateTextTooLong { .. })
        ));
    }

    #[test]
    fn rejects_too_many_fields_and_invalid_colors() {
        let many_fields = (0..11).fold(Embed::builder(), |builder, index| {
            builder.field(EmbedField::new(format!("f{index}"), "v").unwrap())
        });
        assert!(matches!(
            many_fields.build(),
            Err(EmbedValidationError::TooManyFields { actual: 11 })
        ));

        assert!(matches!(
            Embed::builder().color(0x1_00_00_00).build(),
            Err(EmbedValidationError::InvalidColor { .. })
        ));
    }
}
