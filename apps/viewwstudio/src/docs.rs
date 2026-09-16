//! The reference, compiled into the binary.
//!
//! # Why the studio carries its own documentation
//!
//! The studio's whole premise is that you learn a framework by changing
//! something and watching it change — and a person doing that has a question
//! every two minutes, none of which is worth a browser tab: *what does `screen`
//! have to be called, is `Flexible` the one that grows, can I just write a
//! normal function here.* Every one of those answers already existed, in a
//! repository the person who downloaded a `.dmg` does not have.
//!
//! So the pages live beside this file and are `include_str!`d. A packaged
//! studio has them; an offline studio has them; and they are Markdown files a
//! contributor can edit in a pull request rather than strings in Rust.
//!
//! # Why they are rendered rather than opened
//!
//! [`vieww_widget::Markdown`] is a widget in the framework the studio is built
//! from, so the docs view is a dozen lines and the framework gets used by its
//! own tool — which is the cheapest way to find out that a widget is wrong.
//!
//! # What belongs here, and what does not
//!
//! Five short pages that answer the questions of the first hour. Not an API
//! reference: `cargo doc` writes a better one than anybody would maintain by
//! hand here, and a half-copy of it in a sidebar would go stale in a week. The
//! rule for adding a page is that somebody asked the question twice.

/// One page of the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// What the list calls it.
    pub title: &'static str,
    /// One line under the title, so the list is scannable without opening
    /// anything.
    pub summary: &'static str,
    /// The Markdown, verbatim from `docs/`.
    pub body: &'static str,
}

/// The reference, in reading order.
///
/// Reading order, not alphabetical: somebody who opens this view has just
/// arrived, and the first page is the one that gets a screen on the right.
pub const PAGES: [Page; 5] = [
    Page {
        title: "Your first screen",
        summary: "The function Render looks for, and the edit loop.",
        body: include_str!("../docs/01-first-screen.md"),
    },
    Page {
        title: "The shapes",
        summary: "Container, Flex, Stack, Text, Button, ListView.",
        body: include_str!("../docs/02-widgets.md"),
    },
    Page {
        title: "Ordinary Rust",
        summary: "Your own functions and tests, and the one limit on a preview.",
        body: include_str!("../docs/03-rust.md"),
    },
    Page {
        title: "The preview",
        summary: "Platforms, devices, and what is simulated rather than real.",
        body: include_str!("../docs/04-preview.md"),
    },
    Page {
        title: "Onto a device",
        summary: "Desktop, Android, iOS — and what signs a release.",
        body: include_str!("../docs/05-build.md"),
    },
];

/// The page at `index`, or the first.
#[must_use]
pub fn page(index: usize) -> &'static Page {
    PAGES.get(index).unwrap_or(&PAGES[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_something_in_it() {
        // An `include_str!` of a file somebody moved compiles to an empty
        // string in no version of Rust — it fails to compile — but a page
        // emptied by a bad edit would ship silently.
        for page in PAGES {
            assert!(
                page.body.len() > 200,
                "{} is {} bytes, which is not a page",
                page.title,
                page.body.len()
            );
            assert!(
                page.body.starts_with("# "),
                "{} does not start with a heading",
                page.title
            );
        }
    }

    /// The sidebar is narrow, and a fenced code block does not wrap.
    ///
    /// Every other block in a Markdown page is reflowing text. A code block
    /// must not reflow — breaking a line of Rust at whatever column the
    /// container ends at changes what it says — so its intrinsic width is its
    /// longest line, and a page with a 62-character line in it overflowed the
    /// Docs view's row by 228 points and logged that once per frame.
    ///
    /// `Markdown` now clips, so the overflow is contained rather than painted
    /// over the editor. That makes the failure invisible instead of fixing it:
    /// clipped code is *unreadable* code, which for a reference page is the
    /// same defect wearing a tidier coat. The pages have to fit.
    ///
    /// Forty-six is what the sidebar holds at its default width in the body
    /// style. Wrap the line by hand — as the pages do — rather than raising it.
    #[test]
    fn no_code_line_is_wider_than_the_sidebar() {
        const LIMIT: usize = 46;

        for page in PAGES {
            let mut fenced = false;
            for (number, line) in page.body.lines().enumerate() {
                if line.starts_with("```") {
                    fenced = !fenced;
                    continue;
                }
                assert!(
                    !fenced || line.chars().count() <= LIMIT,
                    "{}, line {}: {} characters of code in a {LIMIT}-character \
                     column — wrap it, or it is clipped and cannot be read\n  {line}",
                    page.title,
                    number + 1,
                    line.chars().count(),
                );
            }
            assert!(!fenced, "{} has an unclosed code fence", page.title);
        }
    }

    #[test]
    fn the_pages_answer_the_question_they_were_added_for() {
        // The one somebody asked out loud: *can I write my own Rust, or only
        // vieww components?* If that answer ever leaves the docs, the view has
        // stopped doing the job it was built for.
        let rust = PAGES
            .iter()
            .find(|page| page.title == "Ordinary Rust")
            .expect("the page is in the list");
        assert!(rust.body.contains("normal Rust"));
        assert!(
            rust.body.contains("Build and Run"),
            "and says where the preview's limit stops applying"
        );
    }
}
