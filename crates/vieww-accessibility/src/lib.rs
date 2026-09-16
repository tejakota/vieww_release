//! Automated accessibility verification: can this tree actually be used
//! without sight, and does this palette actually read?
//!
//! # Why this is a crate of its own, downstream of render and widget
//!
//! `vieww-render`'s `SemanticsTree` and the AccessKit bridge in
//! `vieww-platform-winit` get an application in front of a screen reader at
//! all — they are the plumbing. Neither one can tell an author that a button
//! forgot its label, that a tap target shrank to ten pixels, or that a theme's
//! error colour is unreadable on its own background: that is a question about
//! the *content* of a tree or a palette, asked after the fact, and it belongs
//! next to a test suite rather than inside the render loop that builds the
//! tree every frame. Keeping it a separate, downstream crate also keeps it
//! honest about what it is: `vieww-render` and `vieww-widget` do not know this
//! crate exists, so nothing here can quietly become load-bearing for either of
//! them — an audit that stopped compiling would fail exactly one crate's
//! tests, not the framework's core render path.
//!
//! # What "verification" means here, precisely
//!
//! Everything in this crate is a pure function: data in, findings out, no
//! window, no GPU, no screen reader in the room. [`contrast`] hand-computes
//! the published WCAG relative-luminance and contrast-ratio formulas — reusing
//! `vieww-foundation`'s own sRGB linearisation rather than duplicating it —
//! and the pass/fail thresholds that everything else in this crate is judged
//! against. [`audit()`](audit::audit) walks a *built* `SemanticsTree` and flags missing
//! labels, undersized touch targets, and a small set of self-contradictory
//! field combinations, all found from fields that tree actually carries — see
//! that module's own docs for a check that was asked for and honestly not
//! implemented, because the data to back it does not exist on this tree.
//! [`theme_audit`] runs [`contrast`]'s math over the named colour pairs a
//! `vieww-widget::ColorScheme` documents as belonging together, and found in
//! the process that two of the framework's own built-in schemes — the Apple
//! light and dark ones — failed seven of those pairs between them. Those
//! schemes have since been moved onto Apple's own published
//! increased-contrast colours and pass; see that module's docs for what
//! changed and why pinning the failures was not a good enough answer.
//!
//! # What this crate is not
//!
//! It does not run a real screen reader against a real window, does not know
//! whether a colour additionally fails for someone with a specific colour
//! vision deficiency (a different, non-WCAG question from contrast), and does
//! not check keyboard-only operability beyond the one contradiction the
//! semantics tree can actually attest to. Each of those is a real, larger
//! project; claiming to cover them here would be exactly the "wrong-looking
//! pseudocode" this codebase's own conventions ask authors not to write.

pub mod audit;
pub mod contrast;
pub mod theme_audit;

pub use audit::{audit, Finding, Severity, MIN_TOUCH_TARGET_SIDE};
pub use contrast::{contrast_ratio, passes, relative_luminance, WcagLevel};
pub use theme_audit::{
    audit_color_scheme, audit_theme, ThemeAuditReport, ThemeContrastFailure, MIN_NON_TEXT_CONTRAST,
};
