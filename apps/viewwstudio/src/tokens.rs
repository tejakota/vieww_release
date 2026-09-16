//! N8: the theme's tokens, as a list somebody can read, change and export.
//!
//! # What a token editor is for, and what this one refuses to be
//!
//! `ThemeData` is the one place a vieww application's colours are allowed to
//! live — `HTMLPROTOTYPEMAPPING.md` §6 states the rule as *"no control
//! hard-codes a colour or a size; every value in the prototype's shared files
//! must be a custom property"*. That makes the token set the most consequential
//! twenty lines in any project built on this, and until now there was no way to
//! **see** it: you read `theme.rs`, changed a hex by hand, rebuilt, and looked.
//!
//! So this is a list of every token with its current value, an edit that takes
//! effect in the previewed screen immediately, and an export that writes the
//! result as Rust.
//!
//! **It is not a colour picker.** A picker needs a gamut surface, a hue ring,
//! drag handles and an eyedropper — four controls the studio does not have and
//! would have to grow for one view. What it has instead is the edit people
//! actually make: *lighter*, *darker*, and *back to where it was*. A designer
//! choosing a brand colour does it somewhere else and pastes the hex; a
//! developer using this is asking "is this contrast enough", and lightness is
//! that question's axis.
//!
//! # Export writes Rust, and does not rewrite yours
//!
//! [`module`] produces a complete `theme.rs`. It does **not** try to edit an
//! existing one: rewriting somebody's source by pattern-matching hex literals
//! is the kind of thing that works on the file it was tested against and eats a
//! comment on the next one. The studio writes it beside the original and says
//! where, which leaves the merge to the person who knows what else is in there.
//!
//! `std` only, like `edit_ops`, `folding`, `picker` and `scaffold`, so the
//! whole of it runs under `ci/standalone.sh` with no GPU and no font stack.

/// Which part of the theme a token belongs to.
///
/// Grouped exactly as `ThemeData` is, so writing one back is one field per row
/// rather than a mapping table — the prototype's rule for this view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// `ColorScheme`: what a *previewed application* is themed with.
    Colors,
    /// `StudioTheme`: the studio's own chrome. Editable because a token editor
    /// that could not change the thing it is drawn in would be a strange one,
    /// and because the chrome is the fastest way to see a value at size.
    Chrome,
    /// One colour per token class the Rust highlighter emits.
    Syntax,
}

impl Group {
    pub const ALL: [Self; 3] = [Self::Colors, Self::Chrome, Self::Syntax];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Colors => "Colours",
            Self::Chrome => "Chrome",
            Self::Syntax => "Syntax",
        }
    }
}

/// What a token is *for*, which is what decides the bar its contrast is held to.
///
/// # Why the editor needs this
///
/// WCAG has two thresholds and they are for different things: 4.5:1 for body
/// text, 3.0:1 for a control's boundary, and *nothing at all* for a background,
/// which by definition is the thing contrast is measured against. Judging every
/// token by the text bar reported `surface_variant` — a container one step
/// above the editor surface, and correct at 1.2:1 — as **fails**, in red, on a
/// value the studio had deliberately chosen. A checker that cries wolf on
/// correct values is a checker people learn to scroll past, which costs the
/// rows that are genuinely wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Something is read on top of this. 4.5:1.
    Text,
    /// A hairline, an outline, a focus ring: seen but not read. 3.0:1.
    Boundary,
    /// A background. Contrast is reported and not judged — a surface that
    /// stands out from the surface behind it is a *choice about depth*, and the
    /// step between two chrome levels is deliberately small.
    Surface,
}

impl Role {
    /// The ratio this role has to reach, if it has to reach one.
    #[must_use]
    pub const fn bar(self) -> Option<f32> {
        match self {
            Self::Text => Some(4.5),
            Self::Boundary => Some(3.0),
            Self::Surface => None,
        }
    }
}

/// One token: where it lives, what it is called, what it is for, and what it is
/// now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub group: Group,
    /// The field name in `ThemeData`, so the row names the thing to change.
    pub name: &'static str,
    /// `0xRRGGBB`, without alpha. Every token in a theme is opaque; a
    /// translucent one is a decision made at the point of use.
    pub rgb: u32,
    /// What the token is for. Decides how its contrast is judged.
    pub role: Role,
}

/// A colour, adjusted along the one axis this editor offers.
///
/// # Why lightness and not a picker
///
/// See the module header. The step is multiplicative rather than additive so
/// that a dark colour and a light one move by proportionate amounts — adding a
/// fixed 16 to each channel turns `#101014` grey and leaves `#F0F0F4`
/// untouched, which is the wrong way round for both.
///
/// Saturating at 0 and 255 rather than wrapping, because a token that goes
/// round the top and comes back black is a token nobody can nudge confidently.
#[must_use]
pub fn lightened(rgb: u32, factor: f32) -> u32 {
    let channel = |shift: u32| -> u32 {
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a channel is 0..=255 and the result is clamped back into it"
        )]
        {
            let value = ((rgb >> shift) & 0xFF) as f32;
            // A pure black nudged lighter has to leave zero, or it never moves.
            let raised = if value < 8.0 && factor > 1.0 {
                8.0
            } else {
                value * factor
            };
            raised.clamp(0.0, 255.0) as u32
        }
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// `#RRGGBB`, upper case, which is how every design tool writes one.
#[must_use]
pub fn hex(rgb: u32) -> String {
    format!("#{:06X}", rgb & 0x00FF_FFFF)
}

/// Relative luminance, 0.0 to 1.0, by the WCAG definition.
///
/// Here so a row can say whether a colour is light or dark without a human
/// squinting at a swatch, and so [`contrast`] has something to work from.
#[must_use]
pub fn luminance(rgb: u32) -> f32 {
    let channel = |shift: u32| -> f32 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a channel is 0..=255, exactly representable in f32"
        )]
        let value = ((rgb >> shift) & 0xFF) as f32 / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.0722f32.mul_add(
        channel(0),
        0.2126f32.mul_add(channel(16), 0.7152 * channel(8)),
    )
}

/// The WCAG contrast ratio between two colours, 1.0 to 21.0.
///
/// # Why a token editor computes this
///
/// Because the edit this view exists for is almost always *"is this readable
/// on that"*, and the answer is a number with an agreed threshold — 4.5 for
/// body text, 3.0 for large text and interface elements. A swatch beside a
/// swatch is two colours; a ratio is an answer.
#[must_use]
pub fn contrast(a: u32, b: u32) -> f32 {
    let (first, second) = (luminance(a), luminance(b));
    let (lighter, darker) = if first > second {
        (first, second)
    } else {
        (second, first)
    };
    (lighter + 0.05) / (darker + 0.05)
}

/// What a contrast ratio means for a token in `role`, in the guideline's words.
///
/// A [`Role::Surface`] gets no verdict at all — the ratio alone, because the
/// question "does this background contrast with that background" has no pass
/// mark and inventing one turns every deliberate six-point depth step into a
/// failure.
#[must_use]
pub fn contrast_verdict(ratio: f32, role: Role) -> &'static str {
    match role {
        Role::Surface => "",
        Role::Boundary => {
            if ratio >= 3.0 {
                "AA non-text"
            } else {
                "below 3:1"
            }
        }
        Role::Text => {
            if ratio >= 7.0 {
                "AAA"
            } else if ratio >= 4.5 {
                "AA"
            } else if ratio >= 3.0 {
                "AA large only"
            } else {
                "fails"
            }
        }
    }
}

/// The edited token set, as a `theme.rs` somebody can put in a project.
///
/// Complete and compiling rather than a fragment: a snippet that has to be
/// merged into an existing `impl` is a snippet whose braces are the reader's
/// problem.
#[must_use]
pub fn module(tokens: &[Token]) -> String {
    let of = |group: Group| -> Vec<&Token> {
        tokens.iter().filter(|token| token.group == group).collect()
    };

    let mut out = String::from(
        "//! The theme, exported from vieww Studio's token editor.\n\
         //!\n\
         //! Every colour an application draws should come from here — a literal\n\
         //! at a call site is a value that cannot follow the theme it is in.\n\
         \n\
         use vieww::prelude::*;\n\
         use vieww_foundation::Color;\n\
         \n\
         /// This project's colours.\n\
         #[must_use]\n\
         pub fn colors() -> ColorScheme {\n\
         \x20   ColorScheme {\n",
    );
    for token in of(Group::Colors) {
        out.push_str(&format!(
            "        {}: Color::hex(0x{:06X}),\n",
            token.name,
            token.rgb & 0x00FF_FFFF
        ));
    }
    out.push_str("        ..ColorScheme::dark()\n    }\n}\n");

    for (group, heading) in [
        (
            Group::Chrome,
            "The studio chrome these were edited against.",
        ),
        (Group::Syntax, "One colour per highlighter token class."),
    ] {
        let rows = of(group);
        if rows.is_empty() {
            continue;
        }
        out.push_str(&format!("\n/// {heading}\n///\n"));
        out.push_str(
            "/// Exported for reference: these are the studio's own, not this\n\
             /// project's, and nothing here reads them.\n\
             pub mod exported {\n",
        );
        for token in rows {
            out.push_str(&format!(
                "    pub const {}: u32 = 0x{:06X};\n",
                token.name.to_uppercase(),
                token.rgb & 0x00FF_FFFF
            ));
        }
        out.push_str("}\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_written_the_way_a_design_tool_writes_one() {
        assert_eq!(hex(0x00FF_00AB), "#FF00AB");
        assert_eq!(hex(0x0000_0000), "#000000");
        // Alpha in the top byte is not part of a token and is dropped.
        assert_eq!(hex(0xFF12_3456), "#123456");
    }

    #[test]
    fn lightening_is_proportional_and_saturates() {
        let mid = lightened(0x80_8080, 1.25);
        assert_eq!(mid, 0xA0_A0A0);
        // Already at the top: it stays there rather than wrapping to black.
        assert_eq!(lightened(0xFF_FFFF, 1.25), 0xFF_FFFF);
        assert_eq!(lightened(0x00_0000, 0.8), 0x00_0000);
    }

    /// A multiplicative step cannot move pure black, so it is nudged off zero.
    /// Without this the darkest token in a theme is the one that cannot be
    /// edited, which is the opposite of useful.
    #[test]
    fn black_can_still_be_lightened() {
        assert!(lightened(0x00_0000, 1.25) > 0);
    }

    #[test]
    fn contrast_matches_the_guideline_at_its_two_endpoints() {
        // Black on white is the maximum the formula produces.
        let extreme = contrast(0x00_0000, 0xFF_FFFF);
        assert!((extreme - 21.0).abs() < 0.01, "{extreme}");
        assert!((contrast(0x77_7777, 0x77_7777) - 1.0).abs() < 0.001);
        // And it does not care which way round the two are given.
        assert!((contrast(0x00_0000, 0xFF_FFFF) - contrast(0xFF_FFFF, 0x00_0000)).abs() < 0.001);
    }

    #[test]
    fn the_verdict_uses_the_thresholds_the_guideline_names() {
        assert_eq!(contrast_verdict(21.0, Role::Text), "AAA");
        assert_eq!(contrast_verdict(7.0, Role::Text), "AAA");
        assert_eq!(contrast_verdict(4.5, Role::Text), "AA");
        assert_eq!(contrast_verdict(3.0, Role::Text), "AA large only");
        assert_eq!(contrast_verdict(2.9, Role::Text), "fails");
    }

    /// The bar a token is held to is the bar for what it is *for*. A container
    /// one step above the surface is 1.2:1 and correct; the same number for a
    /// label is unreadable.
    #[test]
    fn a_surface_is_reported_and_not_judged() {
        assert_eq!(contrast_verdict(1.2, Role::Surface), "");
        assert_eq!(contrast_verdict(21.0, Role::Surface), "");
        assert_eq!(contrast_verdict(3.1, Role::Boundary), "AA non-text");
        assert_eq!(contrast_verdict(2.4, Role::Boundary), "below 3:1");
        assert_eq!(Role::Surface.bar(), None);
        assert_eq!(Role::Text.bar(), Some(4.5));
        assert_eq!(Role::Boundary.bar(), Some(3.0));
    }

    #[test]
    fn the_export_is_a_whole_module_rather_than_a_fragment() {
        let tokens = [
            Token {
                group: Group::Colors,
                name: "primary",
                rgb: 0x5B_4FD6,
                role: Role::Surface,
            },
            Token {
                group: Group::Colors,
                name: "surface",
                rgb: 0x12_1116,
                role: Role::Surface,
            },
            Token {
                group: Group::Syntax,
                name: "keyword",
                rgb: 0xC5_92BF,
                role: Role::Text,
            },
        ];
        let source = module(&tokens);
        assert!(source.starts_with("//!"), "no header: {source}");
        assert!(source.contains("pub fn colors() -> ColorScheme {"));
        assert!(source.contains("primary: Color::hex(0x5B4FD6),"));
        assert!(source.contains("surface: Color::hex(0x121116),"));
        assert!(source.contains("pub const KEYWORD: u32 = 0xC592BF;"));
        // Balanced braces, which is the one thing a generated module has to
        // get right for the person pasting it.
        assert_eq!(
            source.matches('{').count(),
            source.matches('}').count(),
            "{source}"
        );
    }

    #[test]
    fn a_group_with_no_tokens_writes_no_empty_module() {
        let tokens = [Token {
            group: Group::Colors,
            name: "primary",
            rgb: 1,
            role: Role::Surface,
        }];
        assert!(!module(&tokens).contains("pub mod exported"));
    }
}
