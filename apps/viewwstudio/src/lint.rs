//! Framework-specific diagnostics: the mistakes only a vieww editor can catch.
//!
//! # Why this exists
//!
//! The HTML prototype modelled three diagnostic producers. The studio had two —
//! `rustc` through the build, and `rust-analyzer` through the LSP — and the
//! third, `vieww-lint`, did not exist as a crate, a module, or a reference
//! anywhere in the workspace.
//!
//! Both of the real producers are *general* Rust tools. Neither knows what a
//! `Signal` is, what a `build` method promises, or why a panic inside one is
//! different from a panic anywhere else. The class of mistake this catches is
//! the class that compiles cleanly, passes clippy, and then makes the
//! application spin at sixty frames a second or die on a user's machine.
//!
//! # Why the rules are few, and why they are these
//!
//! A lint that fires often on correct code is a lint people turn off, and a
//! lint layer nobody trusts is worse than none — the Problems panel is where
//! the studio says "something is wrong", and diluting it costs more than the
//! rules are worth. So every rule here has to meet three tests:
//!
//! 1. **It is about vieww**, not about Rust. Anything `clippy` already says is
//!    not this module's business.
//! 2. **It is decidable from the syntax tree.** No type information, no
//!    cross-file reasoning, no guessing — the parse is what there is.
//! 3. **The false-positive story is understood and stated** on the rule.
//!
//! Four rules meet all three today. Adding a fifth is a change here and a test
//! beside it, and the bar is the same.
//!
//! # Where it runs
//!
//! Over the tree-sitter tree the highlighter already builds, on demand rather
//! than per keystroke — so the cost of the whole layer on a frame where nobody
//! asked is zero.

use tree_sitter::Node;

use crate::state::Severity;

/// One thing worth saying about a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Which rule fired. Shown so a rule can be recognised and argued with.
    pub rule: &'static str,
    pub severity: Severity,
    /// One sentence, in the same voice as the studio's other refusals: what is
    /// wrong, and what to do instead.
    pub message: String,
    /// One-based, like every other line number the studio shows.
    pub line: u32,
    pub column: u32,
}

/// Every rule, by name, with what it is for.
///
/// Public so the Problems panel can list what is being checked — a diagnostic
/// producer whose rule set is invisible is one users cannot calibrate against.
pub const RULES: [(&str, &str); 4] = [
    (
        "signal-write-in-build",
        "Writing a signal from inside build rebuilds forever",
    ),
    (
        "panic-in-build",
        "A panic inside build takes the frame down",
    ),
    (
        "print-in-build",
        "Printing from a build runs once per frame",
    ),
    (
        "build-called-directly",
        "The framework calls build; calling it yourself skips the element tree",
    ),
];

/// Lint `source`, which is assumed to be Rust.
///
/// Returns findings in source order. An unparseable file produces none rather
/// than an error: `rustc` is the thing that should complain about syntax, and
/// two producers saying the same thing in different words is how a Problems
/// panel becomes noise.
#[must_use]
pub fn check(source: &str) -> Vec<Finding> {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    check_tree(&tree, source)
}

/// The same rules, over a tree somebody else already built.
///
/// # Why this is the entry point the studio uses
///
/// The module header says these rules run "over the tree-sitter tree the
/// highlighter already builds". They did not: [`check`] made its own `Parser`,
/// set the language, and reparsed the entire buffer — beside a highlighter
/// holding a parse of the very same bytes. Every lint pass on a large file
/// therefore cost a second full parse for nothing, and the doc describing the
/// design was describing something that had never been wired.
///
/// [`check`] stays for callers with only a string — the tests below, and
/// anything checking a file that is not open — and now goes through here.
#[must_use]
pub fn check_tree(tree: &tree_sitter::Tree, source: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    walk(tree.root_node(), source, false, &mut out);
    out.sort_by_key(|finding| (finding.line, finding.column));
    out
}

/// Walk the tree, tracking whether we are inside a `build` method.
///
/// `in_build` is the whole of the context these rules need, which is what
/// keeps this a syntax pass rather than an analysis.
fn walk(node: Node<'_>, source: &str, in_build: bool, out: &mut Vec<Finding>) {
    let in_build = in_build || is_build_fn(node, source);

    if in_build {
        check_in_build(node, source, out);
    }
    check_anywhere(node, source, out);

    // **A closure is not `build`, and this is the false positive that mattered
    // most.** Every one of these rules is about work that happens *while the
    // frame is being built*. A closure written inside `build` and handed to
    // `on_tap` runs when somebody taps, which is exactly where a signal write
    // belongs — and the rule flagged it, on the most common pattern in the
    // framework, in every tutorial, in the studio's own sample screen. A new
    // user's first working screen came up with warnings on the lines the docs
    // had just told them to write.
    //
    // The old text said the honest thing was to "name the limitation rather
    // than pretend the syntax pass can see through a closure's lifetime". It
    // does not need to see through one: the boundary itself is in the tree, and
    // crossing it is the answer. What is given up is a signal write in a
    // closure that genuinely does run during build (`Iterator::map` inside a
    // `children!`), which is rare and, unlike the tap handler, is not what the
    // documentation teaches.
    let in_build = in_build && node.kind() != "closure_expression";

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, in_build, out);
    }
}

/// Whether `node` is the `build` method of a widget.
///
/// # By name *and* by the impl block it is in
///
/// This used to be by name alone, with the false positive stated rather than
/// fixed: "a free function of your own called `build`". That is not a rare
/// name — a builder type's `build()` is the single most common method in Rust —
/// and every one of them collected three warnings about a frame it has nothing
/// to do with.
///
/// The parse *can* answer this. tree-sitter gives the enclosing `impl_item` and
/// its `trait` field, so the question becomes "is this `fn build` inside
/// `impl <something Widget-ish> for …`", which is decidable from the syntax and
/// therefore inside this module's own rules.
///
/// Matching on the trait's *name* rather than resolving it is deliberate: the
/// parse has no idea which crate a path came from, and a user's own
/// `impl MyWidget for …` is not what these rules are about. `Widget`,
/// `StatefulWidget` and `State` are the three the framework defines, and the
/// last path segment is compared so `vieww::Widget` matches too.
fn is_build_fn(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "function_item" {
        return false;
    }
    let named_build = node
        .child_by_field_name("name")
        .and_then(|name| text_of(name, source))
        .is_some_and(|name| name == "build");
    if !named_build {
        return false;
    }
    let mut at = node.parent();
    while let Some(ancestor) = at {
        if ancestor.kind() == "impl_item" {
            return ancestor
                .child_by_field_name("trait")
                .and_then(|trait_| text_of(trait_, source))
                .is_some_and(|trait_| {
                    let last = trait_
                        .rsplit("::")
                        .next()
                        .unwrap_or(trait_)
                        .split('<')
                        .next()
                        .unwrap_or(trait_)
                        .trim();
                    matches!(last, "Widget" | "StatefulWidget" | "State")
                });
        }
        at = ancestor.parent();
    }
    false
}

fn check_in_build(node: Node<'_>, source: &str, out: &mut Vec<Finding>) {
    match node.kind() {
        "call_expression" => {
            let Some(function) = node.child_by_field_name("function") else {
                return;
            };
            let Some(text) = text_of(function, source) else {
                return;
            };

            // **The rule this module is really for.** A build that writes a
            // signal marks the thing it just read as dirty, which schedules
            // another build, which writes the signal again. It compiles, it
            // passes clippy, and the application pins a core at sixty frames a
            // second with nothing visibly wrong.
            //
            // False positive: `something.set(..)` on a type that is not a
            // `Signal` — a `HashSet`, a builder. Stated as a warning for that
            // reason, and the message names the condition rather than accusing.
            if text.ends_with(".set") || text.ends_with(".update") {
                push(
                    out,
                    node,
                    source,
                    "signal-write-in-build",
                    Severity::Warning,
                    "If this is a signal, writing it from `build` schedules another \
                     build, which writes it again. Move the write to an event handler.",
                );
            }

            // A panic during build is caught by the studio's `ErrorPolicy` and
            // shown as a placeholder — and is *not* caught in a shipped
            // application, where the default policy lets it unwind and the
            // process is gone.
            if text.ends_with(".unwrap") || text.ends_with(".expect") {
                push(
                    out,
                    node,
                    source,
                    "panic-in-build",
                    Severity::Warning,
                    "A panic here takes the frame down. The studio shows a placeholder; \
                     a shipped app does not. Return a fallback widget instead.",
                );
            }
        }
        "macro_invocation" => {
            let Some(name) = node
                .child_by_field_name("macro")
                .and_then(|macro_| text_of(macro_, source))
            else {
                return;
            };
            match name {
                "panic" | "unreachable" | "todo" | "unimplemented" => push(
                    out,
                    node,
                    source,
                    "panic-in-build",
                    Severity::Warning,
                    "A panic here takes the frame down. The studio shows a placeholder; \
                     a shipped app does not. Return a fallback widget instead.",
                ),
                "println" | "eprintln" | "print" | "dbg" => push(
                    out,
                    node,
                    source,
                    "print-in-build",
                    Severity::Warning,
                    "A build runs on every frame that touches this widget, so this \
                     prints sixty times a second.",
                ),
                _ => {}
            }
        }
        _ => {}
    }
}

fn check_anywhere(node: Node<'_>, source: &str, out: &mut Vec<Finding>) {
    if node.kind() != "call_expression" {
        return;
    }
    let Some(function) = node.child_by_field_name("function") else {
        return;
    };
    let Some(text) = text_of(function, source) else {
        return;
    };
    // `.build(ctx)` on a widget. The framework calls `build`; calling it
    // yourself produces a `WidgetNode` that never enters the element tree, so
    // it has no state, no keys and no lifecycle — it renders once and is never
    // updated again.
    //
    // False positive: any builder-pattern `.build()` in the wild, which is
    // common. Narrowed to calls that take an argument, because the trait's
    // signature is `build(&self, ctx)` and a builder's is `build()`.
    if text.ends_with(".build") && call_has_arguments(node) {
        push(
            out,
            node,
            source,
            "build-called-directly",
            Severity::Warning,
            "If this is a widget, the framework calls `build` — a node built by hand \
             never enters the element tree and so has no state or lifecycle.",
        );
    }
}

fn call_has_arguments(node: Node<'_>) -> bool {
    node.child_by_field_name("arguments")
        .is_some_and(|arguments| arguments.named_child_count() > 0)
}

fn text_of<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.start_byte()..node.end_byte())
}

fn push(
    out: &mut Vec<Finding>,
    node: Node<'_>,
    source: &str,
    rule: &'static str,
    severity: Severity,
    message: &str,
) {
    let start = node.start_position();
    // Guard against a node whose bytes are not in the source we were handed,
    // which cannot happen for a tree parsed from it and costs one comparison.
    if node.start_byte() > source.len() {
        return;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a file with four billion lines is not one this lints"
    )]
    out.push(Finding {
        rule,
        severity,
        message: message.to_owned(),
        line: start.row as u32 + 1,
        column: start.column as u32 + 1,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_fired(source: &str) -> Vec<&'static str> {
        check(source).into_iter().map(|f| f.rule).collect()
    }

    /// The rule this module is really for: a build that writes a signal
    /// schedules another build, forever. It compiles and it passes clippy.
    #[test]
    fn writing_a_signal_from_build_is_flagged() {
        let source = "
impl Widget for S {
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        self.count.set(1);
        SizedBox::shrink().into()
    }
}
";
        assert!(rules_fired(source).contains(&"signal-write-in-build"));
    }

    /// And the same write outside a build is ordinary code.
    #[test]
    fn writing_a_signal_from_a_handler_is_not_flagged() {
        let source = "
fn on_tap(&self) {
    self.count.set(1);
}
";
        assert!(rules_fired(source).is_empty(), "{:?}", check(source));
    }

    #[test]
    fn a_panic_inside_build_is_flagged_however_it_is_spelled() {
        for body in [
            "self.thing.unwrap();",
            "self.thing.expect(\"here\");",
            "panic!(\"no\");",
            "todo!();",
            "unimplemented!();",
            "unreachable!();",
        ] {
            let source = format!(
                "impl Widget for S {{ fn build(&self, c: &C) -> W {{ {body} W::new() }} }}"
            );
            assert!(
                rules_fired(&source).contains(&"panic-in-build"),
                "not flagged: {body}"
            );
        }
    }

    #[test]
    fn printing_from_a_build_is_flagged() {
        let source =
            "impl Widget for S { fn build(&self, c: &C) -> N { println!(\"hi\"); N::new() } }";
        assert!(rules_fired(source).contains(&"print-in-build"));
    }

    /// Narrowed to calls with arguments, because the trait's signature takes a
    /// context and a builder's `build()` does not.
    #[test]
    fn calling_build_by_hand_is_flagged_and_a_builder_is_not() {
        assert!(
            rules_fired("fn f() { let n = thing.build(ctx); }").contains(&"build-called-directly")
        );
        assert!(
            rules_fired("fn f() { let n = Builder::new().build(); }").is_empty(),
            "an argument-less build() is a builder"
        );
    }

    /// A lint that fires on correct code is a lint people turn off.
    #[test]
    fn ordinary_widget_code_is_silent() {
        let source = "
impl Widget for Screen {
    fn debug_name(&self) -> &'static str { \"Screen\" }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let count = self.count.get();
        Flex::column()
            .children(children![
                Text::new(format!(\"{count}\")),
                Pressable::new(|_| SizedBox::shrink().into())
                    .on_tap({
                        let count = self.count.clone();
                        move || count.set(count.get() + 1)
                    }),
            ])
            .into()
    }
}
";
        // **Silent, including the `set` inside `on_tap`.** That write used to
        // be flagged — it is lexically inside `build` — which meant the
        // framework's most common pattern warned on itself in every tutorial.
        // The walk now stops treating a closure body as build code.
        let fired = rules_fired(source);
        assert!(fired.is_empty(), "correct widget code is silent: {fired:?}");
    }

    #[test]
    fn a_signal_write_in_a_tap_handler_is_not_a_build_write() {
        let source = "
impl Widget for S {
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        Pressable::new(|_| SizedBox::shrink().into())
            .on_tap(move || self.count.set(1))
            .into()
    }
}
";
        assert!(rules_fired(source).is_empty(), "{:?}", check(source));
    }

    #[test]
    fn a_builders_own_build_method_is_not_a_widgets() {
        // The other admitted false positive: `is_build_fn` matched by name, so
        // every builder type in the project collected build-time warnings.
        let source = "
impl Config {
    fn build(&self) -> Config {
        self.dirty.set(true);
        println!(\"built\");
        self.inner.unwrap()
    }
}
";
        assert!(rules_fired(source).is_empty(), "{:?}", check(source));
    }

    #[test]
    fn a_widgets_build_is_still_found_through_a_qualified_trait_path() {
        let source = "
impl vieww::Widget for S {
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        self.count.set(1);
        SizedBox::shrink().into()
    }
}
";
        assert!(rules_fired(source).contains(&"signal-write-in-build"));
    }

    #[test]
    fn an_empty_or_unparseable_file_produces_nothing() {
        assert!(check("").is_empty());
        assert!(check("}}}{{{ not rust at all").is_empty());
    }

    #[test]
    fn findings_come_back_in_source_order() {
        let source = "
impl Widget for S {
    fn build(&self, c: &C) -> N {
        println!(\"one\");
        panic!(\"two\");
    }
}
";
        let findings = check(source);
        assert!(findings.len() >= 2);
        for pair in findings.windows(2) {
            assert!(
                (pair[0].line, pair[0].column) <= (pair[1].line, pair[1].column),
                "{findings:?}"
            );
        }
    }

    #[test]
    fn a_finding_points_at_a_real_line() {
        let source =
            "impl Widget for S {\n    fn build(&self, c: &C) -> N {\n        panic!();\n    }\n}";
        let findings = check(source);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].line, 3);
        assert!(findings[0].column > 1, "and at a real column");
    }

    /// Every rule in `RULES` has to be one `check` can actually produce, or the
    /// list the Problems panel shows is a promise the linter does not keep.
    #[test]
    fn every_advertised_rule_is_reachable() {
        let sources = [
            "impl Widget for S { fn build(&self, c: &C) -> N { x.set(1); N } }",
            "impl Widget for S { fn build(&self, c: &C) -> N { panic!(); N } }",
            "impl Widget for S { fn build(&self, c: &C) -> N { println!(\"x\"); N } }",
            "fn f() { thing.build(ctx); }",
        ];
        let mut seen: Vec<&str> = Vec::new();
        for source in sources {
            seen.extend(rules_fired(source));
        }
        for (rule, _) in RULES {
            assert!(seen.contains(&rule), "{rule} is advertised and unreachable");
        }
    }

    #[test]
    fn every_rule_has_a_description() {
        for (rule, description) in RULES {
            assert!(!rule.is_empty());
            assert!(!description.is_empty(), "{rule}");
        }
    }
}
