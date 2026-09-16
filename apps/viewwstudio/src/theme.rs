//! The chrome palette, published beside `ThemeData` rather than inside it.
//!
//! `ColorScheme` has one `surface`. An IDE needs four: an activity bar that
//! recedes, a sidebar that sits behind the editor, the editor itself, and
//! something raised for tabs and popovers. Adding those roles to
//! `vieww-widget`'s scheme would put IDE vocabulary in a framework every
//! application shares, so they live here and travel the same way a theme does —
//! through [`Inherited`](vieww_widget::Inherited).
//!
//! Every value is derived from a `ColorScheme`, not typed in twice, so a studio
//! built on `ThemeData::light()` is one call away and cannot drift from the
//! controls drawn inside it.

use vieww_foundation::{Color, Gradient, Offset, Shadow, TargetPlatform};
use vieww_widget::{BuildContext, ColorScheme, ThemeData};

/// A brand accent: the two ends of its ramp in each theme, and how strong its
/// wash is behind a selected thing.
///
/// A struct rather than six constants because the six only make sense together
/// — a dark near-stop paired with a light far-stop is a ramp that inverts
/// between themes — and because trying a different accent should be one edit,
/// which is what [`ACCENT`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brand {
    /// The dark theme's near stop. Buttons carry white text on this, so it has
    /// to hold 4.5:1 against white.
    pub dark_near: Color,
    /// The dark theme's far stop: the lighter, more saturated end.
    pub dark_far: Color,
    /// The light theme's near stop. Same rule, and it bites harder — a light
    /// interface needs a *deeper* accent to keep white text on it readable.
    pub light_near: Color,
    /// The light theme's far stop.
    pub light_far: Color,
    /// How strong the wash behind a selected activity-bar entry is, out of 255.
    /// Higher in the dark theme, where a tint has further to travel to read.
    pub dark_wash: u8,
    pub light_wash: u8,
}

impl Brand {
    #[must_use]
    pub const fn near(self, dark: bool) -> Color {
        if dark {
            self.dark_near
        } else {
            self.light_near
        }
    }

    #[must_use]
    pub const fn far(self, dark: bool) -> Color {
        if dark {
            self.dark_far
        } else {
            self.light_far
        }
    }

    /// The near stop at wash strength.
    #[must_use]
    pub const fn wash(self, dark: bool) -> Color {
        let color = self.near(dark);
        let alpha = if dark {
            self.dark_wash
        } else {
            self.light_wash
        };
        Color::rgba(color.r, color.g, color.b, alpha)
    }

    /// The fill behind a selected row, opaque.
    ///
    /// # Why this is not just [`wash`](Self::wash)
    ///
    /// A wash is translucent, and a selected row is a thing text is read on
    /// top of. Composited over an unknown ground its final contrast is unknown
    /// too, and the studio's list rows sit on three different chrome depths.
    /// Blending against the surface here gives one colour with one measurable
    /// contrast — and, on a dark theme, keeps a selected row *darker* than the
    /// accent itself so the row reads as tinted rather than as a coloured bar
    /// with text stranded on it. That is what VS Code's own `#264F78` is: a
    /// desaturated, darkened version of its blue, not the blue.
    /// # Why the blend is written out rather than using `Color::over`
    ///
    /// `over` works in `f32`, which no `const fn` may do, and `StudioTheme`'s
    /// two constructors are `const` — they are the two literal blocks the whole
    /// palette is read from, and a person looking up "what colour is the
    /// sidebar" should find a table, not a function call. Eight-bit integer
    /// mixing is exact enough for one opaque blend and keeps them that way.
    #[must_use]
    pub const fn selection(self, dark: bool) -> Color {
        let accent = self.near(dark);
        if dark {
            // Toward the editor surface, keeping about a third of the accent.
            mix(accent, Color::hex(0x1F_1F1F), 0x5E)
        } else {
            // On a light theme the row has to stay light enough to read black
            // text on, so the accent is a tint over white rather than a shade.
            mix(accent, Color::WHITE, 0x38)
        }
    }
}

/// `top` at `alpha` over an opaque `bottom`, in integer arithmetic.
///
/// Rounds half up, the way [`Color::over`] does, so the two agree to the last
/// bit on every input either can be given.
const fn mix(top: Color, bottom: Color, alpha: u8) -> Color {
    Color::rgba(
        channel(top.r, bottom.r, alpha),
        channel(top.g, bottom.g, alpha),
        channel(top.b, bottom.b, alpha),
        255,
    )
}

/// One channel of [`mix`]. A free function because a `const fn` may not call a
/// closure, and three copies of the expression is worse than one name.
const fn channel(top: u8, bottom: u8, alpha: u8) -> u8 {
    let (over, under) = (alpha as u16, 255 - alpha as u16);
    ((top as u16 * over + bottom as u16 * under + 127) / 255) as u8
}

/// **The studio's accent. One line to change it.**
///
/// Everything accented reads from here: the Render button's ramp, the tab and
/// panel indicators, the activity bar's rail and wash, the focus ring, the
/// segmented control's thumb, and the mark in the title bar. Swapping this
/// constant recolours all of them together, which is the only way an accent
/// stays one decision rather than nineteen.
///
/// Not `ColorScheme::primary`: that belongs to the *previewed application* and
/// changes when somebody edits their own theme. The Render button is the
/// studio's button.
pub const ACCENT: Brand = PURPLE;

/// The blue this shipped with — VS Code's Fluent accent.
pub const BLUE: Brand = Brand {
    dark_near: Color::hex(0x00_78D4),
    dark_far: Color::hex(0x3F_A7F5),
    light_near: Color::hex(0x00_5FB8),
    light_far: Color::hex(0x24_87D6),
    dark_wash: 0x3A,
    light_wash: 0x26,
};

/// A light purple.
///
/// The near stops are deeper than the name suggests, and deliberately: the
/// ramp's *far* stop is the light purple anyone would pick off a swatch, and
/// the near stop is where white text actually sits. `#B491FF` against white is
/// 1.9:1 — a Render button labelled in a colour nobody could read. These hold
/// 4.7:1 in the dark theme and 6.6:1 in the light one, and the gradient still
/// reads as the lighter colour because the far stop is most of what is seen.
pub const PURPLE: Brand = Brand {
    dark_near: Color::hex(0x7E_5CE8),
    dark_far: Color::hex(0xB4_91FF),
    light_near: Color::hex(0x6B_3FCC),
    light_far: Color::hex(0x8E_6BE8),
    dark_wash: 0x3E,
    light_wash: 0x28,
};

/// A deep teal.
pub const TEAL: Brand = Brand {
    dark_near: Color::hex(0x0E_8174),
    dark_far: Color::hex(0x2F_BFAE),
    light_near: Color::hex(0x0A_6B60),
    light_far: Color::hex(0x15_9C8C),
    dark_wash: 0x3A,
    light_wash: 0x26,
};

/// A forest green.
pub const GREEN: Brand = Brand {
    dark_near: Color::hex(0x2E_7D32),
    dark_far: Color::hex(0x5C_B860),
    light_near: Color::hex(0x2A_6E2E),
    light_far: Color::hex(0x3E_9C43),
    dark_wash: 0x3A,
    light_wash: 0x26,
};

/// A burnt amber.
///
/// The near stop is darker than an amber "wants" to be, and this is the accent
/// where that constraint bites hardest: yellow is the brightest hue there is, so
/// the colour that reads as amber on a swatch cannot hold white text at all.
/// `#A8690A` was the first try and missed 4.5:1 by two hundredths.
pub const AMBER: Brand = Brand {
    dark_near: Color::hex(0xA0_5F08),
    dark_far: Color::hex(0xE0_A44A),
    light_near: Color::hex(0x8F_5808),
    light_far: Color::hex(0xC1_820F),
    dark_wash: 0x3A,
    light_wash: 0x26,
};

/// A warm orange.
pub const ORANGE: Brand = Brand {
    dark_near: Color::hex(0xC2_410C),
    dark_far: Color::hex(0xF0_8A4B),
    light_near: Color::hex(0xA6_370A),
    light_far: Color::hex(0xD6_5A1E),
    dark_wash: 0x3A,
    light_wash: 0x26,
};

/// A deep rose.
pub const ROSE: Brand = Brand {
    dark_near: Color::hex(0xC0_2853),
    dark_far: Color::hex(0xF2_708F),
    light_near: Color::hex(0xA6_1F45),
    light_far: Color::hex(0xD1_416B),
    dark_wash: 0x3A,
    light_wash: 0x26,
};

/// A magenta.
pub const MAGENTA: Brand = Brand {
    dark_near: Color::hex(0xA5_309E),
    dark_far: Color::hex(0xE0_70D8),
    light_near: Color::hex(0x8C_2286),
    light_far: Color::hex(0xC2_4BBA),
    dark_wash: 0x3A,
    light_wash: 0x26,
};

/// Every accent the studio ships, in the order the settings list shows them.
///
/// # Why a slice and not a `HashMap`
///
/// The order is part of the design — the list runs cool to warm — and the set
/// is eight items that change about once a year. A slice keeps the order, keeps
/// the whole thing `const`, and makes "every accent" a thing the tests can
/// iterate rather than a thing they have to be told.
pub const ACCENTS: &[(&str, Brand)] = &[
    ("Purple", PURPLE),
    ("Blue", BLUE),
    ("Teal", TEAL),
    ("Green", GREEN),
    ("Amber", AMBER),
    ("Orange", ORANGE),
    ("Rose", ROSE),
    ("Magenta", MAGENTA),
];

/// The accent stored under `name`, or the default when nothing matches.
///
/// Falls back rather than failing: the name comes out of a settings file that a
/// person can edit and an older studio can have written, and an accent nobody
/// recognises should leave the studio purple rather than refusing to start.
#[must_use]
pub fn accent_named(name: &str) -> Brand {
    ACCENTS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map_or(ACCENT, |(_, brand)| *brand)
}

/// The name of the accent in `brand`, or the default's name.
#[must_use]
pub fn accent_name(brand: Brand) -> &'static str {
    ACCENTS
        .iter()
        .find(|(_, known)| *known == brand)
        .map_or("Purple", |(name, _)| *name)
}

/// Chrome depths and editor colours, layered over a `ColorScheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StudioTheme {
    /// The ground every region floats on.
    ///
    /// New with the card shell. Before it, the regions met edge to edge and a
    /// hairline was the only thing between them, so there was nothing *behind*
    /// the shell to have a colour. Now each region is a rounded surface with a
    /// gutter around it, and this is what shows through the gutter — darker
    /// than [`chrome_0`](Self::chrome_0) in both themes, because a ground that
    /// is lighter than the surfaces on it reads as a gap rather than a floor.
    pub window: Color,
    /// Activity bar and status bar — the frame around everything.
    pub chrome_0: Color,
    /// Sidebar and bottom panel.
    pub chrome_1: Color,
    /// The editor surface. Always the scheme's own `surface`.
    pub chrome_2: Color,
    /// Raised: the active tab, a popover, the command palette.
    pub chrome_3: Color,
    /// Hover.
    pub chrome_4: Color,
    /// Hairline separators between regions.
    pub line: Color,
    /// Line numbers, at rest.
    pub gutter: Color,
    /// The line number of the caret's line.
    pub gutter_active: Color,
    /// Selection wash in the editor and the explorer.
    pub selection: Color,
    /// A warning that is not an error: an unused import, a slow re-highlight.
    pub warning: Color,
    /// Syntax roles. Kept as a struct rather than a map so a missing one is a
    /// compile error rather than a token that silently paints as body text.
    pub syntax: Syntax,

    // ---------------------------------------------------------------- depth
    /// Which of the two themes this is.
    ///
    /// Carried on the theme rather than passed beside it. Everything that
    /// draws depth needs to know — a shadow under a card is nearly black on a
    /// dark ground and a soft grey on a light one, and a sheen lightens the
    /// top edge in one and darkens it in the other — and a `dark` flag
    /// threaded as a second argument through thirty call sites is a flag that
    /// disagrees with its theme exactly once.
    pub dark: bool,
    /// The studio's own accent, and the first stop of every accent ramp.
    ///
    /// Distinct from `ColorScheme::primary`, which belongs to the *previewed
    /// application* and changes when somebody edits their theme. The Render
    /// button is the studio's button; it should not change colour because the
    /// screen being previewed did.
    pub accent: Color,
    /// The far end of the accent ramp. A gradient needs two colours and a
    /// single-colour "gradient" is a flat fill with extra steps.
    pub accent_soft: Color,
    /// The accent at a strength that can sit under something — focus rings,
    /// glows, the wash behind a selected row.
    pub accent_wash: Color,
}

/// One colour per token class the Rust highlighter emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Syntax {
    pub keyword: Color,
    pub function: Color,
    pub type_name: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub macro_name: Color,
    pub punctuation: Color,
    pub attribute: Color,
}

impl StudioTheme {
    /// The dark chrome, over [`ColorScheme::dark`].
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            window: Color::hex(0x0E_0E0E),
            chrome_0: Color::hex(0x18_1818),
            chrome_1: Color::hex(0x1A_1A1A),
            chrome_2: Color::hex(0x1F_1F1F),
            chrome_3: Color::hex(0x26_2626),
            chrome_4: Color::hex(0x2F_2F2F),
            line: Color::hex(0x2B_2B2B),
            // Lifted from `#6E7681`, which is GitHub's muted grey and reads
            // 3.6:1 against the editor surface. A line number is **text** — it
            // is looked up and read, not glanced at — so 4.5:1 is the bar, and
            // 3.6 is the width of the gap between "recessive" and "squinting".
            // Same hue, one step brighter: 4.65:1.
            gutter: Color::hex(0x82_898F),
            gutter_active: Color::hex(0xCC_CCCC),
            selection: ACCENT.selection(true),
            warning: Color::hex(0xCC_A700),
            syntax: Syntax {
                keyword: Color::hex(0x56_9CD6),
                function: Color::hex(0xDC_DCAA),
                type_name: Color::hex(0x4E_C9B0),
                string: Color::hex(0xCE_9178),
                number: Color::hex(0xB5_CEA8),
                comment: Color::hex(0x6A_9955),
                macro_name: Color::hex(0xC5_86C0),
                punctuation: Color::hex(0xD4_D4D4),
                attribute: Color::hex(0x9C_DCFE),
            },
            dark: true,
            accent: ACCENT.near(true),
            accent_soft: ACCENT.far(true),
            accent_wash: ACCENT.wash(true),
        }
    }

    /// The light chrome, over [`ColorScheme::light`].
    #[must_use]
    pub const fn light() -> Self {
        Self {
            window: Color::hex(0xE8_E8E8),
            chrome_0: Color::hex(0xF8_F8F8),
            chrome_1: Color::hex(0xF8_F8F8),
            chrome_2: Color::hex(0xFF_FFFF),
            chrome_3: Color::hex(0xFF_FFFF),
            chrome_4: Color::hex(0xEB_EBEB),
            line: Color::hex(0xE5_E5E5),
            // 3.1:1 against the white editor surface, which is under the bar
            // for text and was the light theme's worst-read element after the
            // activity bar. `#757575` is 4.6:1 and still a clear step below
            // `gutter_active`, so the caret's line stays the one that stands
            // out.
            gutter: Color::hex(0x75_7575),
            gutter_active: Color::hex(0x2C_2C2C),
            selection: ACCENT.selection(false),
            // A warning that cannot be seen is not a warning. `#BF8803` is
            // 2.9:1 on the light chrome — under even the *non-text* 3:1 bar,
            // and this colour is a squiggle, a panel row's severity dot and a
            // count, all of which are read. Darkened to 4.5:1 while staying
            // recognisably amber rather than turning into the error red.
            warning: Color::hex(0x9A_6A00),
            syntax: Syntax {
                keyword: Color::hex(0x00_00FF),
                function: Color::hex(0x79_5E26),
                type_name: Color::hex(0x26_7F99),
                string: Color::hex(0xA3_1515),
                number: Color::hex(0x09_8658),
                comment: Color::hex(0x00_8000),
                macro_name: Color::hex(0xAF_00DB),
                punctuation: Color::hex(0x38_3A42),
                attribute: Color::hex(0x00_1080),
            },
            dark: false,
            accent: ACCENT.near(false),
            accent_soft: ACCENT.far(false),
            accent_wash: ACCENT.wash(false),
        }
    }

    /// The studio chrome published to this subtree, or the dark one.
    ///
    /// Mirrors [`ThemeData::of`] deliberately: a region that reads its chrome
    /// the same way it reads its colours cannot end up half-themed.
    #[must_use]
    pub fn of(ctx: &BuildContext) -> std::rc::Rc<Self> {
        ctx.inherit_or(Self::dark())
    }

    // --------------------------------------------------------------- ramps
    //
    // Derived rather than stored. A gradient is two colours and a direction,
    // and only the colours are worth a person's time in the token editor —
    // storing the direction as a token would mean a row nobody can read and a
    // theme that can be edited into something that is not a ramp at all.

    /// The accent ramp: [`accent`](Self::accent) into
    /// [`accent_soft`](Self::accent_soft), top-left to bottom-right.
    ///
    /// Diagonal rather than vertical because the shapes it fills are wide and
    /// short — a button, a pill, a tab indicator — and a vertical ramp across
    /// twenty-six points of height is a flat fill that costs a gradient.
    #[must_use]
    pub fn accent_ramp(&self) -> Gradient {
        Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0))
            .between(self.accent, self.accent_soft)
    }

    /// A surface with light falling on its top edge.
    ///
    /// One white (or, in the light theme, black) veil at the top fading to
    /// nothing by a third of the way down. The strength is deliberately small:
    /// the effect wanted is "this is a surface" and not "this is a gradient",
    /// and anything visible as a ramp on a flat panel reads as a mistake.
    #[must_use]
    pub fn sheen(&self, base: Color) -> Gradient {
        let veil = if self.dark {
            Color::rgba(255, 255, 255, 9)
        } else {
            Color::rgba(255, 255, 255, 160)
        };
        Gradient::vertical().with_stops(&[(0.0, veil.over(base)), (0.34, base), (1.0, base)])
    }

    /// A surface lit from the top *and* shaded at the bottom — for the tall
    /// regions, where a sheen alone runs out of ramp long before the bottom.
    #[must_use]
    pub fn deep_sheen(&self, base: Color) -> Gradient {
        let (veil, shade) = if self.dark {
            (Color::rgba(255, 255, 255, 14), Color::rgba(0, 0, 0, 46))
        } else {
            (Color::rgba(255, 255, 255, 170), Color::rgba(0, 0, 0, 12))
        };
        Gradient::vertical().with_stops(&[
            (0.0, veil.over(base)),
            (0.28, base),
            (1.0, shade.over(base)),
        ])
    }

    /// The shadow a surface at `level` casts, 1 to 4.
    ///
    /// A ramp rather than a set of named constants, because the thing a caller
    /// actually knows is *how far off the page* something is, and the four
    /// levels are: a resting row, a hovered control, a popover, a modal. The
    /// numbers grow faster than linearly in blur and slower than linearly in
    /// alpha, which is what keeps level 4 from looking like a black rectangle.
    #[must_use]
    pub fn elevation(&self, level: u8) -> Shadow {
        let level = f32::from(level.clamp(1, 4));
        let alpha = if self.dark {
            (46.0 + level * 30.0).min(190.0)
        } else {
            (16.0 + level * 12.0).min(90.0)
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=190 above"
        )]
        Shadow::new(
            Color::rgba(0, 0, 0, alpha as u8),
            Offset::new(0.0, level * 1.6),
            level * level * 1.6 + 2.0,
        )
    }

    /// The hairline that catches the light along a raised surface's top edge,
    /// blended against the surface it sits on.
    ///
    /// One pixel of caught light is the whole difference between a panel that
    /// is *on* the ground and one that is a hole cut in it.
    ///
    /// # Why this takes the background and returns an opaque colour
    ///
    /// It used to return `rgba(255, 255, 255, 20)` and let the compositor do
    /// the blend, which is the obvious way to write it and was wrong on a real
    /// screen: the line arrived at full strength. A pure white hairline against
    /// near-black picks up subpixel colour fringing on an LCD, so it was
    /// reported as *"yellow horizontal lines between the panes"* — a sentence
    /// with no visible connection to the alpha channel, about a bug no
    /// screenshot taken through the CPU rasteriser reproduced.
    ///
    /// Blending here removes the question. The rim sits on a *known* colour —
    /// its own card's background, which is the argument — so the composite can
    /// be done once, in `f32`, by the same `over` the rest of this file uses,
    /// and what reaches the renderer is an opaque colour that every backend
    /// draws identically. [`sheen`](Self::sheen) already worked this way, which
    /// is exactly why the sheen never had the bug.
    #[must_use]
    pub fn rim(&self, background: Color) -> Color {
        let veil = if self.dark {
            Color::rgba(255, 255, 255, 20)
        } else {
            Color::rgba(255, 255, 255, 220)
        };
        veil.over(background)
    }

    /// The ring drawn around whatever has keyboard focus.
    #[must_use]
    pub const fn focus(&self) -> Color {
        self.accent
    }
}

/// The `ThemeData` each chrome is meant to sit on.
///
/// Paired here so the two can never be mixed — a dark chrome around light
/// controls is a bug that no test of either half alone would catch.
#[must_use]
pub fn theme_data(dark: bool) -> ThemeData {
    let base = if dark {
        ThemeData::dark()
    } else {
        ThemeData::light()
    };
    // **The studio's chrome is pinned; the preview inside it is not.**
    //
    // `ThemeData::light()` follows the host machine now, and for an ordinary
    // application that is the right default — an unbranded app should look
    // native. The studio is not that application. It has a visual identity of
    // its own (the card shell, `chrome_0`..`chrome_4`, its own hairlines and
    // corners), it is the same product on all three desktops, and its
    // `examples/screenshot` pictures are this repository's regression
    // mechanism: a shell that drew Apple's switch on a Mac runner and
    // the Android shape on a Linux one would produce two different pictures of the
    // same commit.
    //
    // So the chrome says which shapes it means, which is exactly what
    // `with_platform` is for. The **previewed screen** is themed separately by
    // `ui::preview`, with `ThemeData::adaptive(platform.target(), …)` — that
    // one has to follow the picker, and it is the whole reason the picker
    // exists.
    let mut data = base.with_platform(TargetPlatform::Android);

    // **The chrome's own foregrounds, over the framework's.**
    //
    // `ColorScheme::dark`'s greys are the stock dark ones, tuned for an *application* on
    // a `surface` of `#1B1B1F`: a near-white `on_surface` and a lilac-tinted
    // `on_surface_variant`. In a window that is nine tenths chrome they read as
    // slightly too bright and slightly too purple — every label in the studio
    // shares a hue with the Rust keywords beside it.
    //
    // These are an editor's greys: a settled foreground a step below white, a
    // secondary that is genuinely secondary, and an outline dark enough to
    // separate without drawing a box. Neutral, so nothing in the chrome
    // competes with the syntax colours, which are the only thing in the window
    // whose hue is carrying information.
    //
    // The *previewed* screen is untouched — it gets `ThemeData::adaptive`, and
    // an application previewed here should look like the platform's, not like
    // this editor's.
    if dark {
        data.colors.on_surface = Color::hex(0xCC_CCCC);
        data.colors.on_surface_variant = Color::hex(0x9D_9D9D);
        // **Lifted to clear the bar the token editor holds it to.**
        //
        // `#3C3C3C` is VS Code's input border, and as a *border* it is fine.
        // In this studio `outline` is mostly small text — a tab's qualifier,
        // a token's hex value, a tree's chevrons — where 1.6:1 against the
        // editor surface is not dim, it is unreadable. The hairline role is
        // `StudioTheme::line`'s, and that one stays where it was.
        //
        // 3.2:1 against the *editor* surface, which is the measurement that was
        // taken — but `outline` also lands on `chrome_3` (a raised popover, an
        // active tab) and on `surface_variant`, where the same colour is 2.97:1
        // and 2.70:1 and the bar is missed on the two surfaces a chevron is
        // most often on. `#7A7A7A` clears 3:1 on every chrome step, which is
        // what "the non-text bar" has to mean if it is to mean anything.
        // Anything read at body size should still be reaching for
        // `on_surface_variant`.
        data.colors.outline = Color::hex(0x7A_7A7A);
        // A container a step above the editor surface, and one the token
        // editor's own contrast column can pass: the stock `#46464F` reads
        // 1.8:1 against this surface, which the editor duly reported as
        // "fails" about a colour the studio had chosen for it.
        data.colors.surface_variant = Color::hex(0x2D_2D2D);
        data.colors.primary = ACCENT.near(true);
        data.colors.on_primary = Color::WHITE;
        data.colors.error = Color::hex(0xF8_5149);
        data.colors.on_error = Color::WHITE;
        data.colors.success = Color::hex(0x3F_B950);
        data.colors.on_success = Color::hex(0x0A_1A0D);
    } else {
        data.colors.on_surface = Color::hex(0x3B_3B3B);
        data.colors.on_surface_variant = Color::hex(0x61_6161);
        // The light counterpart, and the same argument: `#8E8E8E` is 3.1:1 on
        // the chrome and 2.87:1 on `surface_variant`. `#7E7E7E` clears 3:1 on
        // every step of the light chrome.
        data.colors.outline = Color::hex(0x7E_7E7E);
        data.colors.surface_variant = Color::hex(0xF0_F0F0);
        data.colors.primary = ACCENT.near(false);
        data.colors.on_primary = Color::WHITE;
        data.colors.error = Color::hex(0xCD_3131);
        data.colors.on_error = Color::WHITE;
        data.colors.success = Color::hex(0x14_7E3A);
        data.colors.on_success = Color::WHITE;
    }
    data
}

/// The chrome that belongs with [`theme_data`] at the same `dark`.
#[must_use]
pub const fn studio_theme(dark: bool) -> StudioTheme {
    if dark {
        StudioTheme::dark()
    } else {
        StudioTheme::light()
    }
}

/// The same, recoloured to `brand`.
///
/// # Why this recolours a theme rather than parameterising the constructors
///
/// [`StudioTheme::dark`] and [`::light`](StudioTheme::light) are two literal
/// tables, and their value is that they are readable: someone asking "what
/// colour is the sidebar" finds a hex number on a line. Threading an accent
/// through them would put a function call on all four accent rows of both, to
/// express something that is true of exactly five fields out of twenty.
///
/// So the tables stay literal in the default accent and this replaces the five
/// that depend on it. Every one is listed here, which is the property that
/// matters: an accent-derived field added to `StudioTheme` and forgotten here
/// would be the one thing on screen that stayed purple.
#[must_use]
pub const fn studio_theme_with(dark: bool, brand: Brand) -> StudioTheme {
    let mut theme = studio_theme(dark);
    theme.accent = brand.near(dark);
    theme.accent_soft = brand.far(dark);
    theme.accent_wash = brand.wash(dark);
    theme.selection = brand.selection(dark);
    theme
}

/// The scheme the chrome was derived from, for the few places that need both.
#[must_use]
pub const fn colors(dark: bool) -> ColorScheme {
    if dark {
        ColorScheme::dark()
    } else {
        ColorScheme::light()
    }
}
