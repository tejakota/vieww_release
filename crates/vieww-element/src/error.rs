use std::any::Any;
use std::fmt;

use vieww_widget::{ErrorPlaceholder, WidgetNode};

use crate::ElementId;

/// What the tree does when a widget's `build` panics.
///
/// The default is [`Placeholder`](ErrorPolicy::Placeholder) in a debug build and
/// [`Propagate`](ErrorPolicy::Propagate) in a release one — the roadmap's
/// framing, and the conservative reading of it. Catching a panic is a debugging
/// convenience: it keeps one broken screen from taking the process down while
/// somebody is looking at it. In release the same catch would hide a bug behind
/// a magenta box on a user's phone, and a crash report is worth more than that.
///
/// An application that disagrees can say so with
/// [`ElementTree::set_error_policy`](crate::ElementTree::set_error_policy).
#[derive(Debug, Clone, Copy)]
pub enum ErrorPolicy {
    /// Catch the panic, mount an
    /// [`ErrorPlaceholder`](vieww_widget::ErrorPlaceholder) in place of what the
    /// build would have produced, and record a [`BuildError`].
    Placeholder,
    /// The same, with a widget of the application's own.
    ///
    /// ```
    /// use vieww_element::{BuildError, ErrorPolicy};
    /// use vieww_widget::{Text, WidgetNode};
    ///
    /// fn apology(error: &BuildError) -> WidgetNode {
    ///     Text::new(format!("Sorry — {} could not be shown", error.widget_name)).into()
    /// }
    ///
    /// let policy = ErrorPolicy::Custom(apology);
    /// ```
    ///
    /// # What this is for
    ///
    /// The built-in placeholder is a magenta box, which is right for a developer
    /// and wrong for anybody else. An application wanting its own branding, its
    /// own message formatting, or a "report this" button had no way in: the
    /// element tree constructed `ErrorPlaceholder` directly and recognised it by
    /// **type** on the way back.
    ///
    /// # The recursion guard still holds, and now holds for you too
    ///
    /// Whatever this returns is built like any other widget, so a substitute
    /// that panics would be replaced by another substitute for ever. Say
    /// [`Widget::catches_panics`](vieww_widget::Widget::catches_panics) `->
    /// false` on it, exactly as `ErrorPlaceholder` does. That guard used to be a
    /// downcast to vieww's own type, which protected vieww's own placeholder and
    /// nothing else.
    ///
    /// A `fn` pointer rather than a boxed closure, so the policy stays `Copy`
    /// and comparable — the same trade `Curve::Custom` makes, and it means a
    /// placeholder cannot capture state.
    Custom(fn(&BuildError) -> WidgetNode),
    /// Let the panic unwind out of the build, taking the frame — and in almost
    /// every configuration the process — with it.
    Propagate,
}

/// # Hand-written because a derive cannot compare a function pointer honestly
///
/// `#[derive(PartialEq)]` over [`Custom`](Self::Custom) compares the pointer
/// with `==`, which rustc warns about
/// (`unpredictable_function_pointer_comparisons`): one function can have
/// different addresses in different codegen units, and two functions can share
/// an address once the linker merges identical bodies.
///
/// [`std::ptr::fn_addr_eq`] asks the same question in the sanctioned way, with
/// the same caveats. It is identity, approximately — which is all there is to
/// compare, since two placeholder builders that produce the same widget are
/// still not the same builder.
impl PartialEq for ErrorPolicy {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Placeholder, Self::Placeholder) | (Self::Propagate, Self::Propagate) => true,
            (Self::Custom(ours), Self::Custom(theirs)) => std::ptr::fn_addr_eq(*ours, *theirs),
            _ => false,
        }
    }
}

/// Sound because [`fn_addr_eq`](std::ptr::fn_addr_eq) is reflexive, symmetric
/// and transitive over the addresses it compares, whatever those addresses turn
/// out to be.
impl Eq for ErrorPolicy {}

impl ErrorPolicy {
    /// `true` if this policy catches a panic rather than letting it out.
    #[must_use]
    pub const fn catches(self) -> bool {
        matches!(self, Self::Placeholder | Self::Custom(_))
    }

    /// The widget to mount in place of the build that failed.
    #[must_use]
    pub fn placeholder_for(self, error: &BuildError) -> WidgetNode {
        match self {
            Self::Custom(build) => build(error),
            // `Propagate` never reaches here — `catches` is checked first — and
            // answering with the default placeholder rather than panicking keeps
            // a mistake in that order from turning a caught panic into an
            // uncaught one inside the error path itself.
            Self::Placeholder | Self::Propagate => {
                ErrorPlaceholder::new(error.widget_name, error.message.clone()).into()
            }
        }
    }
}

impl Default for ErrorPolicy {
    fn default() -> Self {
        if cfg!(debug_assertions) {
            Self::Placeholder
        } else {
            Self::Propagate
        }
    }
}

/// A panic caught out of one widget's `build`.
///
/// Recorded on the tree rather than only printed, so a test can assert on it and
/// a frame-timing or inspector overlay can show it. The panic *also* goes to the
/// standard panic hook on its way past, which is where the backtrace is: this
/// type carries what a program can act on, not everything a human wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    /// The element whose build failed. It is still mounted — what was replaced
    /// is the subtree its build would have returned.
    pub element: ElementId,
    /// `debug_name` of the widget that panicked.
    pub widget_name: &'static str,
    /// The panic message, when it was a `&str` or a `String`. Panics carrying
    /// any other payload report only their type.
    pub message: String,
}

impl BuildError {
    /// Pull a readable message out of a panic payload.
    ///
    /// `panic!` produces a `&'static str` for a literal and a `String` for a
    /// formatted message; `panic_any` can produce anything at all, and there is
    /// nothing useful to say about a payload whose type we cannot name.
    pub(crate) fn message_from(payload: &(dyn Any + Send)) -> String {
        if let Some(message) = payload.downcast_ref::<&'static str>() {
            (*message).to_owned()
        } else if let Some(message) = payload.downcast_ref::<String>() {
            message.clone()
        } else {
            String::from("<non-string panic payload>")
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (element {}) panicked during build: {}",
            self.widget_name, self.element, self.message
        )
    }
}
