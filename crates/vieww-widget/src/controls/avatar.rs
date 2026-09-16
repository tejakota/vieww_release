use std::fmt;

use vieww_foundation::{Color, Image as ImageData, Key, TextStyle};

use crate::{
    widget_node_from, BuildContext, Center, Clip, ColoredBox, Image, SemanticRole, Semantics,
    SizedBox, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The side of the square an [`Avatar`] takes when nobody says otherwise.
pub const DEFAULT_AVATAR_SIZE: f32 = 40.0;

/// What an [`Avatar`] shows: a decoded photo, or up to two letters over a
/// solid fill.
#[derive(Clone)]
enum AvatarSource {
    Image(ImageData),
    Initials(String),
}

/// A small circular picture of a person — a photo if there is one, initials
/// over a solid fill if there is not.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Avatar;
///
/// let placeholder = Avatar::initials("Ada Lovelace");
/// ```
///
/// # Why initials takes a name, not the letters themselves
///
/// [`initials`](Self::initials) takes whatever the application already has
/// — a person's full name — rather than asking the caller to extract two
/// letters from it first. The extraction (first letter of the first two
/// words, upper-cased) lives here once instead of being re-implemented
/// slightly differently at every call site.
///
/// # Not itself a [`Badge`](crate::Badge) host
///
/// A status dot or an unread count on top of an avatar is composition, not
/// a feature this widget needs: `Badge::new(Avatar::initials(name)).dot()`
/// already does it, using the corner [`Positioned`](crate::Positioned)
/// already gives `Badge` for free.
#[derive(Clone)]
pub struct Avatar {
    source: AvatarSource,
    size: f32,
    color: Option<Color>,
    key: Option<Key>,
}

impl Avatar {
    /// A decoded photo, cropped to a circle and centred.
    #[must_use]
    pub fn image(image: ImageData) -> Self {
        Self {
            source: AvatarSource::Image(image),
            size: DEFAULT_AVATAR_SIZE,
            color: None,
            key: None,
        }
    }

    /// Up to two letters — the first letter of the first two words of
    /// `name`, upper-cased — over a solid fill.
    ///
    /// A name with one word shows one letter rather than padding a second
    /// from nowhere. An empty name renders an empty circle with no
    /// accessibility label, rather than a "?" or some other placeholder
    /// nobody asked for.
    #[must_use]
    pub fn initials(name: impl AsRef<str>) -> Self {
        let letters: String = name
            .as_ref()
            .split_whitespace()
            .take(2)
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect();

        Self {
            source: AvatarSource::Initials(letters),
            size: DEFAULT_AVATAR_SIZE,
            color: None,
            key: None,
        }
    }

    /// The diameter, in logical pixels. Defaults to
    /// [`DEFAULT_AVATAR_SIZE`].
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Override the fill behind initials. Has no effect on
    /// [`image`](Self::image) — a photo is not tinted.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Avatar {
    fn debug_name(&self) -> &'static str {
        "Avatar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let (content, label): (WidgetNode, String) = match &self.source {
            AvatarSource::Image(image) => (
                Image::new(image.clone()).into(),
                String::new(), // decorative by default; see the type's docs
            ),
            AvatarSource::Initials(letters) => {
                let color = self.color.unwrap_or(theme.colors.primary);
                (
                    ColoredBox::new(color)
                        .child(
                            Center::new().child(Text::new(letters.clone()).style(TextStyle {
                                color: theme.colors.on_primary,
                                ..theme.text.label
                            })),
                        )
                        .into(),
                    letters.clone(),
                )
            }
        };

        let circle = Clip::oval().child(SizedBox::square(self.size).child(content));

        if label.is_empty() {
            circle.into()
        } else {
            Semantics::new()
                .role(SemanticRole::Label)
                .label(label)
                .child(circle)
                .into()
        }
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = match &self.source {
            AvatarSource::Image(_) => vec![("source", "image".to_owned())],
            AvatarSource::Initials(letters) => vec![("initials", letters.clone())],
        };
        if self.size != DEFAULT_AVATAR_SIZE {
            props.push(("size", self.size.to_string()));
        }
        props
    }
}

impl fmt::Debug for Avatar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Avatar")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Avatar);

#[cfg(test)]
mod tests {
    use crate::{inflate, DebugNode, Theme};

    use super::*;

    fn built(avatar: Avatar) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(avatar))
    }

    #[test]
    fn two_words_give_two_upper_case_initials() {
        let node = built(Avatar::initials("ada lovelace"));
        let text = node.find("Text").expect("initials Text node");
        assert_eq!(text.property("text"), Some("\"AL\""));
    }

    #[test]
    fn one_word_gives_one_letter() {
        let node = built(Avatar::initials("Cher"));
        let text = node.find("Text").expect("initials Text node");
        assert_eq!(text.property("text"), Some("\"C\""));
    }

    #[test]
    fn three_or_more_words_still_give_only_two_letters() {
        let node = built(Avatar::initials("Ursula K LeGuin"));
        let text = node.find("Text").expect("initials Text node");
        assert_eq!(text.property("text"), Some("\"UK\""));
    }

    #[test]
    fn empty_input_shows_nothing_and_carries_no_label() {
        let node = built(Avatar::initials(""));
        assert!(node.find("Semantics").is_none());
        assert_eq!(
            node.find("Text").and_then(|n| n.property("text")),
            Some("\"\"")
        );
    }

    #[test]
    fn the_clip_shape_is_always_an_oval() {
        let node = built(Avatar::initials("AB"));
        assert!(node.find("Clip").is_some());
    }
}
