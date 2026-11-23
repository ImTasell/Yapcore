use serde::Serialize;
use thiserror::Error;

use crate::models::{ActionRow, Button};

/// Flag required when sending or editing a Components V2 layout message.
pub const COMPONENTS_V2_FLAG: u64 = 1 << 15;

/// Text display component (Components V2 type 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextDisplay {
    #[serde(rename = "type")]
    component_type: u8,
    content: String,
}

impl TextDisplay {
    /// Create a Markdown text display.
    pub fn new(content: impl Into<String>) -> Result<Self, LayoutValidationError> {
        let content = content.into();
        if content.is_empty() {
            return Err(LayoutValidationError::EmptyText);
        }
        Ok(Self {
            component_type: 10,
            content,
        })
    }

    /// Displayed Markdown content.
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Thumbnail accessory for a Components V2 section (type 11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Thumbnail {
    #[serde(rename = "type")]
    component_type: u8,
    media: MediaReference,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    spoiler: bool,
}

impl Thumbnail {
    /// Create an HTTPS thumbnail accessory.
    pub fn new(url: impl Into<String>) -> Result<Self, LayoutValidationError> {
        let url = url.into();
        validate_https_url(&url)?;
        Ok(Self {
            component_type: 11,
            media: MediaReference { url },
            description: None,
            spoiler: false,
        })
    }

    /// Set alternative text for the thumbnail.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Mark the thumbnail as a spoiler.
    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }
}

/// Accessory displayed beside a section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SectionAccessory {
    /// An interactive button.
    Button(Button),
    /// A thumbnail image.
    Thumbnail(Thumbnail),
}

impl From<Button> for SectionAccessory {
    fn from(value: Button) -> Self {
        Self::Button(value)
    }
}

impl From<Thumbnail> for SectionAccessory {
    fn from(value: Thumbnail) -> Self {
        Self::Thumbnail(value)
    }
}

/// Section with one to three text displays and one accessory (type 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    #[serde(rename = "type")]
    component_type: u8,
    components: Vec<TextDisplay>,
    accessory: SectionAccessory,
}

impl Section {
    /// Create a section from one to three text displays and an accessory.
    pub fn new<I, A>(texts: I, accessory: A) -> Result<Self, LayoutValidationError>
    where
        I: IntoIterator<Item = TextDisplay>,
        A: Into<SectionAccessory>,
    {
        let components = texts.into_iter().collect::<Vec<_>>();
        if components.is_empty() {
            return Err(LayoutValidationError::EmptySection);
        }
        if components.len() > 3 {
            return Err(LayoutValidationError::TooManySectionLines {
                actual: components.len(),
            });
        }
        Ok(Self {
            component_type: 9,
            components,
            accessory: accessory.into(),
        })
    }
}

/// One image or video entry in a media gallery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaGalleryItem {
    media: MediaReference,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    spoiler: bool,
}

impl MediaGalleryItem {
    /// Create an HTTPS media gallery item.
    pub fn new(url: impl Into<String>) -> Result<Self, LayoutValidationError> {
        let url = url.into();
        validate_https_url(&url)?;
        Ok(Self {
            media: MediaReference { url },
            description: None,
            spoiler: false,
        })
    }

    /// Set alternative text for the media item.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Mark this media item as a spoiler.
    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }
}

/// Gallery containing one to ten media items (type 12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaGallery {
    #[serde(rename = "type")]
    component_type: u8,
    items: Vec<MediaGalleryItem>,
}

impl MediaGallery {
    /// Create a media gallery from one to ten items.
    pub fn new<I>(items: I) -> Result<Self, LayoutValidationError>
    where
        I: IntoIterator<Item = MediaGalleryItem>,
    {
        let items = items.into_iter().collect::<Vec<_>>();
        if items.is_empty() {
            return Err(LayoutValidationError::EmptyGallery);
        }
        if items.len() > 10 {
            return Err(LayoutValidationError::TooManyGalleryItems {
                actual: items.len(),
            });
        }
        Ok(Self {
            component_type: 12,
            items,
        })
    }
}

/// Spacing applied by a Components V2 separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorSpacing {
    /// Compact spacing.
    Small,
    /// Larger spacing.
    Large,
}

impl SeparatorSpacing {
    fn value(self) -> u8 {
        match self {
            Self::Small => 1,
            Self::Large => 2,
        }
    }
}

/// Visual separator or spacer (type 14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Separator {
    #[serde(rename = "type")]
    component_type: u8,
    divider: bool,
    spacing: u8,
}

impl Separator {
    /// Create a separator. Set divider to false to render spacing only.
    pub fn new(divider: bool, spacing: SeparatorSpacing) -> Self {
        Self {
            component_type: 14,
            divider,
            spacing: spacing.value(),
        }
    }
}

/// Downloadable HTTPS file component (type 13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDisplay {
    #[serde(rename = "type")]
    component_type: u8,
    file: MediaReference,
    #[serde(skip_serializing_if = "is_false")]
    spoiler: bool,
}

impl FileDisplay {
    /// Create a file component from an HTTPS URL.
    pub fn new(url: impl Into<String>) -> Result<Self, LayoutValidationError> {
        let url = url.into();
        validate_https_url(&url)?;
        Ok(Self {
            component_type: 13,
            file: MediaReference { url },
            spoiler: false,
        })
    }

    /// Mark the file as a spoiler.
    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }
}

/// Container/card holding other Components V2 items (type 17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Container {
    #[serde(rename = "type")]
    component_type: u8,
    components: Vec<LayoutComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accent_color: Option<u32>,
    #[serde(skip_serializing_if = "is_false")]
    spoiler: bool,
}

impl Container {
    /// Create a container from nested layout components.
    pub fn new<I>(components: I) -> Self
    where
        I: IntoIterator<Item = LayoutComponent>,
    {
        Self {
            component_type: 17,
            components: components.into_iter().collect(),
            accent_color: None,
            spoiler: false,
        }
    }

    /// Set the container's accent color as a 24-bit RGB value.
    pub fn accent_color(mut self, color: u32) -> Result<Self, LayoutValidationError> {
        if color > 0x00ff_ffff {
            return Err(LayoutValidationError::InvalidAccentColor);
        }
        self.accent_color = Some(color);
        Ok(self)
    }

    /// Mark the whole container as a spoiler.
    pub fn spoiler(mut self, spoiler: bool) -> Self {
        self.spoiler = spoiler;
        self
    }
}

/// A component allowed at the top level of a Components V2 message or inside a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LayoutComponent {
    /// Button action row.
    ActionRow(ActionRow),
    /// Markdown text.
    TextDisplay(TextDisplay),
    /// Text section with an accessory.
    Section(Section),
    /// Media gallery.
    MediaGallery(MediaGallery),
    /// Divider or spacer.
    Separator(Separator),
    /// Downloadable file.
    FileDisplay(FileDisplay),
    /// Nested card/container.
    Container(Container),
}

impl From<ActionRow> for LayoutComponent {
    fn from(value: ActionRow) -> Self {
        Self::ActionRow(value)
    }
}

impl From<TextDisplay> for LayoutComponent {
    fn from(value: TextDisplay) -> Self {
        Self::TextDisplay(value)
    }
}

impl From<Section> for LayoutComponent {
    fn from(value: Section) -> Self {
        Self::Section(value)
    }
}

impl From<MediaGallery> for LayoutComponent {
    fn from(value: MediaGallery) -> Self {
        Self::MediaGallery(value)
    }
}

impl From<Separator> for LayoutComponent {
    fn from(value: Separator) -> Self {
        Self::Separator(value)
    }
}

impl From<FileDisplay> for LayoutComponent {
    fn from(value: FileDisplay) -> Self {
        Self::FileDisplay(value)
    }
}

impl From<Container> for LayoutComponent {
    fn from(value: Container) -> Self {
        Self::Container(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct MediaReference {
    url: String,
}

/// Validate the top-level layout before sending it to Yapper.
pub(crate) fn validate_layout(
    components: &[LayoutComponent],
) -> Result<(), LayoutValidationError> {
    if components.is_empty() {
        return Err(LayoutValidationError::EmptyLayout);
    }
    Ok(())
}

/// Validation failures for Components V2 layout builders.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutValidationError {
    /// A Components V2 message must contain at least one component.
    #[error("a Components V2 message must contain at least one component")]
    EmptyLayout,
    /// Text display content must not be empty.
    #[error("text display content must not be empty")]
    EmptyText,
    /// A section must contain at least one text display.
    #[error("a section must contain at least one text display")]
    EmptySection,
    /// A section may contain no more than three text displays.
    #[error("a section may contain at most three text displays; received {actual}")]
    TooManySectionLines {
        /// Number of text displays provided.
        actual: usize,
    },
    /// A media gallery must contain at least one item.
    #[error("a media gallery must contain at least one item")]
    EmptyGallery,
    /// A media gallery may contain no more than ten items.
    #[error("a media gallery may contain at most 10 items; received {actual}")]
    TooManyGalleryItems {
        /// Number of media items provided.
        actual: usize,
    },
    /// Media and file URLs must use HTTPS.
    #[error("Components V2 media and file URLs must use HTTPS")]
    InvalidMediaUrl,
    /// An accent color must be a 24-bit RGB value.
    #[error("container accent color must be a 24-bit RGB value")]
    InvalidAccentColor,
}

fn validate_https_url(url: &str) -> Result<(), LayoutValidationError> {
    if url
        .get(..8)
        .is_none_or(|scheme| !scheme.eq_ignore_ascii_case("https://"))
        || url.len() <= 8
    {
        return Err(LayoutValidationError::InvalidMediaUrl);
    }
    Ok(())
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_v2_components_in_upstream_shape() {
        let layout = vec![
            TextDisplay::new("# Welcome").unwrap().into(),
            Section::new(
                [TextDisplay::new("Ready to continue?").unwrap()],
                Button::primary("Continue", "continue").unwrap(),
            )
            .unwrap()
            .into(),
            MediaGallery::new([
                MediaGalleryItem::new("https://cdn.example/image.png")
                    .unwrap()
                    .spoiler(true),
            ])
            .unwrap()
            .into(),
            Separator::new(true, SeparatorSpacing::Large).into(),
            Container::new([TextDisplay::new("Nested").unwrap().into()])
                .accent_color(0x5865f2)
                .unwrap()
                .into(),
        ];

        let value = serde_json::to_value(&layout).unwrap();
        assert_eq!(value[0]["type"], 10);
        assert_eq!(value[0]["content"], "# Welcome");
        assert_eq!(value[1]["type"], 9);
        assert_eq!(value[1]["components"][0]["type"], 10);
        assert_eq!(value[1]["accessory"]["custom_id"], "continue");
        assert_eq!(value[2]["type"], 12);
        assert_eq!(value[2]["items"][0]["media"]["url"], "https://cdn.example/image.png");
        assert_eq!(value[2]["items"][0]["spoiler"].as_bool(), Some(true));
        assert_eq!(value[3]["type"], 14);
        assert_eq!(value[3]["spacing"], 2);
        assert_eq!(value[4]["type"], 17);
        assert_eq!(value[4]["accent_color"], 0x5865f2);
    }

    #[test]
    fn validates_section_gallery_layout_and_media_constraints() {
        assert!(matches!(
            TextDisplay::new(""),
            Err(LayoutValidationError::EmptyText)
        ));
        assert!(matches!(
            Thumbnail::new("http://example.com/image.png"),
            Err(LayoutValidationError::InvalidMediaUrl)
        ));
        assert!(matches!(
            Section::new([], Button::primary("Go", "go").unwrap()),
            Err(LayoutValidationError::EmptySection)
        ));
        assert!(matches!(
            MediaGallery::new([]),
            Err(LayoutValidationError::EmptyGallery)
        ));
        assert!(matches!(
            validate_layout(&[]),
            Err(LayoutValidationError::EmptyLayout)
        ));
    }
}
