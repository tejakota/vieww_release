//! The snippet library, checked for shape.
//!
//! The **items** are checked properly, by compiling them — that is
//! `tests/pipeline.rs::every_item_snippet_compiles`, and it needs a built
//! workspace, so it skips where there is none.
//!
//! An **expression** snippet cannot be compiled in isolation and it would be
//! dishonest to pretend otherwise: it is a fragment meant for a caret, and half
//! of them name things that only exist there — `theme`, `items`, `child`. What
//! can be checked without inventing a context is the shape, and the shape is
//! where a hand-edited snippet actually goes wrong: a brace lost in an edit, or
//! an item filed under expressions, where inserting it inside a `build` is a
//! syntax error in the user's file rather than in ours.

use viewwstudio::edit_ops::{SnippetKind, SNIPPETS};

/// Whether every `(`, `[` and `{` outside a string or a comment is closed.
///
/// A small scanner rather than a parser: strings and line comments are the two
/// things that would make a naive count wrong, and both are cheap to skip.
fn balanced(body: &str) -> bool {
    let chars: Vec<char> = body.chars().collect();
    let mut stack: Vec<char> = Vec::new();
    let mut at = 0usize;

    while at < chars.len() {
        match chars[at] {
            '"' => {
                // Skip the string, honouring escapes.
                at += 1;
                while at < chars.len() {
                    match chars[at] {
                        '\\' => at += 1,
                        '"' => break,
                        _ => {}
                    }
                    at += 1;
                }
                if at >= chars.len() {
                    return false;
                }
            }
            // **An apostrophe is usually a lifetime here, not a literal.**
            // `&'static str` and `WidgetKind<'_>` are both in these bodies, and
            // reading either as the start of a character literal swallows the
            // rest of the snippet — which is how this scanner first reported a
            // perfectly balanced `impl` block as unbalanced. A literal is
            // `'\''`-shaped: an escape, or exactly one character then a quote.
            '\'' => {
                let escaped = chars.get(at + 1) == Some(&'\\');
                let single = chars.get(at + 2) == Some(&'\'');
                if escaped || single {
                    at += 1;
                    while at < chars.len() {
                        match chars[at] {
                            '\\' => at += 1,
                            '\'' => break,
                            _ => {}
                        }
                        at += 1;
                    }
                }
            }
            '/' if chars.get(at + 1) == Some(&'/') => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
                continue;
            }
            '(' | '[' | '{' => stack.push(chars[at]),
            ')' if stack.pop() != Some('(') => return false,
            ']' if stack.pop() != Some('[') => return false,
            '}' if stack.pop() != Some('{') => return false,
            _ => {}
        }
        at += 1;
    }

    stack.is_empty()
}

#[test]
fn every_snippet_is_balanced() {
    for snippet in SNIPPETS {
        assert!(
            balanced(snippet.body),
            "the {} snippet has an unclosed bracket or string:\n{}",
            snippet.name,
            snippet.body
        );
    }
}

#[test]
fn an_expression_snippet_is_not_secretly_an_item() {
    // Filed wrong, this puts an `impl` block in the middle of a `build` — a
    // syntax error in the user's file, produced by a button in ours.
    for snippet in SNIPPETS
        .iter()
        .filter(|snippet| snippet.kind == SnippetKind::Expression)
    {
        let first = snippet
            .body
            .lines()
            .find(|line| !line.trim_start().starts_with("//") && !line.trim().is_empty())
            .unwrap_or_default()
            .trim_start();
        for keyword in ["pub ", "impl ", "fn ", "struct ", "use ", "#[derive"] {
            assert!(
                !first.starts_with(keyword),
                "the {} snippet starts with `{keyword}`, so it is an item, not an expression",
                snippet.name
            );
        }
    }
}

#[test]
fn the_library_leads_with_the_shape_of_a_file() {
    // Ordering is the teaching: somebody who has just arrived needs to know
    // what `screen()` looks like before they need a `Flex::row`. The view
    // groups by kind and lists items first, so this pins the entry that has to
    // be at the top of that group.
    let first_item = SNIPPETS
        .iter()
        .find(|snippet| snippet.kind == SnippetKind::Item)
        .expect("there is at least one item snippet");
    assert_eq!(first_item.name, "Screen entry point");
    assert!(
        first_item.body.contains("pub fn screen() -> impl Widget"),
        "the first snippet is the contract Render looks for"
    );
}

#[test]
fn there_are_function_level_snippets_and_not_only_expressions() {
    // The gap this closed: eleven entries, ten of them expressions to drop
    // inside a `build`, and nothing that showed what a *file* is made of.
    let items = SNIPPETS
        .iter()
        .filter(|snippet| snippet.kind == SnippetKind::Item)
        .count();
    assert!(items >= 5, "only {items} item snippets");
}

#[test]
fn every_snippet_says_where_it_came_from() {
    for snippet in SNIPPETS {
        assert!(!snippet.name.trim().is_empty());
        assert!(
            !snippet.source.trim().is_empty(),
            "{} has no source line, so the row cannot say where to read more",
            snippet.name
        );
    }
}
