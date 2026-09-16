//! Parsing, part two: widget lines and everything that hangs off them —
//! properties, event blocks, actions, conditions and expressions. This file
//! continues the `Parser` from `parse`; the split is only so each half fits
//! in a reader's head.

use crate::diag::codes;
use crate::lex::{tokenize, Tok};
use crate::parse::{
    AKind, ActionLine, Align, BinOp, CmpOp, ColorSpec, Cond, Cross, EventBlock, EventKind, Expr,
    Icon, NamedColor, Parser, Part, Pin, Prop, StateKind, ThemeSlot, Value, WKind, Widget,
};

/// The scalar types expressions and targets carry, after lists are excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ty {
    Whole,
    Number,
    Text,
    Flag,
}

fn scalar(kind: StateKind) -> Option<Ty> {
    match kind {
        StateKind::Whole => Some(Ty::Whole),
        StateKind::Number => Some(Ty::Number),
        StateKind::Text => Some(Ty::Text),
        StateKind::Flag => Some(Ty::Flag),
        StateKind::ListText | StateKind::ListNumber => None,
    }
}

fn ty_name(ty: Ty) -> &'static str {
    match ty {
        Ty::Whole => "a whole number",
        Ty::Number => "a number",
        Ty::Text => "text",
        Ty::Flag => "a yes-or-no",
    }
}

/// What kind of block follows the clauses of a widget line.
enum Block {
    Children,
    Actions(EventKind, Option<String>),
}

impl<'a> Parser<'a> {
    // -- widget lines ------------------------------------------------------

    /// One widget line: phrase, clauses, then the block its trailing colon
    /// opens — children, or the actions of an event.
    pub(crate) fn parse_widget_line(
        &mut self,
        index: usize,
        indent: u32,
    ) -> Option<(Widget, usize)> {
        let (number, toks) = self
            .lines
            .get(index)
            .map(|l| (l.number, l.tokens.clone()))?;
        let word = |c: usize| toks.get(c).and_then(Tok::word);
        let str_at = |c: usize| -> Option<String> {
            toks.get(c).and_then(|t| match t {
                Tok::Str { text, .. } => Some(text.clone()),
                _ => None,
            })
        };

        // The article opens every widget line and carries no meaning — it is
        // grammar, not vocabulary. Skip it.
        if word(0) != Some("a") && word(0) != Some("an") && word(0) != Some("the") {
            self.error(
                number,
                1,
                codes::E060,
                "a widget line starts with `a`, `an` or `the` — or `for each` for a repetition",
            );
            return None;
        }

        let (kind, phrase, mut c) = self.parse_phrase(&toks, number)?;
        let mut props = Vec::new();
        let mut event: Option<EventBlock> = None;
        let mut only_if = None;
        let mut block = Block::Children;

        loop {
            match toks.get(c) {
                None => break,
                Some(Tok::Colon) => {
                    c += 1;
                    break;
                }
                Some(Tok::Comma) => {
                    c += 1;
                }
                Some(Tok::Word(w)) if w == "which" => {
                    // which when tapped|pressed|submitted :
                    let event_word = word(c + 2).unwrap_or_default();
                    let kind = match event_word {
                        "tapped" | "pressed" => EventKind::Tapped,
                        "submitted" => EventKind::Submitted,
                        _ => {
                            self.error(
                                number,
                                1,
                                codes::E070,
                                "an event reads `which when tapped:` (or `pressed`, or, on a field, `submitted`)",
                            );
                            return None;
                        }
                    };
                    if toks.get(c + 3) != Some(&Tok::Colon) {
                        self.error(number, 1, codes::E070, "an event block ends with `:`");
                        return None;
                    }
                    event = Some(EventBlock {
                        kind,
                        label: None,
                        actions: Vec::new(),
                    });
                    block = Block::Actions(kind, None);
                    c += 4;
                }
                Some(Tok::Word(w)) if w == "only" => {
                    let (cond, next) = self.parse_cond(&toks, c + 2, number)?;
                    only_if = Some(cond);
                    c = next;
                }
                Some(Tok::Word(w))
                    if w == "with"
                        && word(c + 1) == Some("the")
                        && word(c + 2) == Some("action") =>
                {
                    // The empty state's action: `with the action "Add one"
                    // which when tapped:` — a label and an event in one.
                    let Some(label) = str_at(c + 3) else {
                        self.error(
                            number,
                            1,
                            codes::E070,
                            "`with the action` carries a quoted label",
                        );
                        return None;
                    };
                    if !(word(c + 4) == Some("which")
                        && word(c + 5) == Some("when")
                        && word(c + 6) == Some("tapped")
                        && toks.get(c + 7) == Some(&Tok::Colon))
                    {
                        self.error(
                            number,
                            1,
                            codes::E070,
                            "an action's event reads `which when tapped:`",
                        );
                        return None;
                    }
                    event = Some(EventBlock {
                        kind: EventKind::Action,
                        label: Some(label.clone()),
                        actions: Vec::new(),
                    });
                    block = Block::Actions(EventKind::Action, Some(label));
                    c += 8;
                }
                Some(_) => {
                    let prop = self.parse_prop(&toks, &mut c, number, &kind, phrase)?;
                    props.push(prop);
                }
            }
        }

        // The block after the clauses: a widget's children, or an event's
        // actions. Only one is possible — an event clause ends the line.
        let (children, next) = match block {
            Block::Children => {
                let opened = c > 0 && toks.get(c - 1) == Some(&Tok::Colon);
                if opened {
                    let (children, next) = self.parse_block(index + 1, indent + 1);
                    (children, next)
                } else {
                    (Vec::new(), index + 1)
                }
            }
            Block::Actions(kind, label) => {
                let (actions, next) = self.parse_actions_block(index + 1, indent + 1);
                if let Some(event) = event.as_mut() {
                    event.actions = actions;
                }
                let _ = (kind, label);
                (Vec::new(), next)
            }
        };

        if children.is_empty() && matches!(kind, WKind::Column | WKind::Row | WKind::Stack) {
            // Legal — an empty box — but nine times in ten it is a colon that
            // was meant to follow a different line. The linter says so rather
            // than guessing.
            self.warn(
                number,
                1,
                codes::W130,
                format!(
                    "this {phrase} has no children; did the block under it lose its indentation?"
                ),
            );
        }

        Some((
            Widget {
                kind,
                props,
                event,
                only_if,
                children,
                line: number,
                phrase,
            },
            next,
        ))
    }

    // -- phrases -----------------------------------------------------------

    /// The widget phrase at the cursor. Longest phrase wins (§8.6 rule 1),
    /// which here means: `a text field …` is tested before `a text …`.
    fn parse_phrase(&mut self, toks: &[Tok], number: u32) -> Option<(WKind, &'static str, usize)> {
        let word = |c: usize| toks.get(c).and_then(Tok::word);
        let str_at = |c: usize| -> Option<String> {
            toks.get(c).and_then(|t| match t {
                Tok::Str { text, .. } => Some(text.clone()),
                _ => None,
            })
        };
        let num_at = |c: usize| -> Option<f32> {
            toks.get(c).and_then(|t| match t {
                Tok::Float(f) => Some(*f),
                Tok::Int(i) => Some(*i as f32),
                _ => None,
            })
        };
        let is = |c: usize, w: &str| word(c) == Some(w);

        let first = word(1).unwrap_or_default().to_owned();
        let mut c;
        let result: Option<(WKind, &'static str)> = match first.as_str() {
            "card" | "box" | "panel" => {
                c = 2;
                Some((WKind::Card, "a card"))
            }
            "column" | "vertical" => {
                c = usize::from(is(2, "line")) + 2;
                Some((WKind::Column, "a column"))
            }
            "row" | "horizontal" => {
                c = usize::from(is(2, "line")) + 2;
                Some((WKind::Row, "a row"))
            }
            "stack" | "layered" => {
                c = usize::from(is(2, "area")) + 2;
                Some((WKind::Stack, "a stack"))
            }
            "center" | "centered" => {
                c = usize::from(is(2, "area")) + 2;
                Some((WKind::Center, "a center"))
            }
            "fixed" | "spacer" => {
                // `a fixed area of W by H` · `a fixed area of N square` ·
                // `a spacer of W by H`.
                c = if first == "fixed" && is(2, "area") {
                    3
                } else {
                    2
                };
                if !is(c, "of") {
                    self.error(
                        number,
                        1,
                        codes::E070,
                        "a fixed area reads `a fixed area of W by H`",
                    );
                    return None;
                }
                c += 1;
                let (width, height) = match (num_at(c), word(c + 1)) {
                    (Some(w), Some("by")) => (Some(w), num_at(c + 2)),
                    (Some(s), Some("square")) => (Some(s), Some(s)),
                    _ => {
                        self.error(
                            number,
                            1,
                            codes::E070,
                            "give the area a size: `of 80 by 40` or `of 80 square`",
                        );
                        return None;
                    }
                };
                c += if is(c + 1, "by") { 3 } else { 2 };
                Some((WKind::Fixed { width, height }, "a fixed area"))
            }
            "text" if is(2, "field") || is(2, "input") => {
                if !is(3, "bound") || !is(4, "to") {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a text field binds to state: `a text field bound to s`",
                    );
                    return None;
                }
                let bound = word(5).unwrap_or_default().to_owned();
                self.check_bound(&bound, number)?;
                c = 6;
                Some((WKind::TextField { bound }, "a text field"))
            }
            "text" | "label" => {
                let Some(text) = str_at(2) else {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a text carries its words: `a text \"Hello\"`",
                    );
                    return None;
                };
                let value = self.value_from_string(text, number)?;
                c = 3;
                Some((
                    WKind::Text {
                        value,
                        heading: false,
                    },
                    "a text",
                ))
            }
            "heading" | "title" => {
                let Some(text) = str_at(2) else {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a heading carries its words: `a heading \"Home\"`",
                    );
                    return None;
                };
                let value = self.value_from_string(text, number)?;
                c = 3;
                Some((
                    WKind::Text {
                        value,
                        heading: true,
                    },
                    "a heading",
                ))
            }
            "button" => {
                let Some(text) = str_at(2) else {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a button carries its label: `a button \"Save\"`",
                    );
                    return None;
                };
                let label = self.value_from_string(text, number)?;
                c = 3;
                Some((WKind::Button { label }, "a button"))
            }
            "switch" => {
                let bound = self.bound_after(toks, 2, number, "a switch")?;
                self.check_bound_state_only(&bound, number, "a switch")?;
                c = 5;
                Some((WKind::Switch { bound }, "a switch"))
            }
            "checkbox" => {
                let bound = self.bound_after(toks, 2, number, "a checkbox")?;
                self.check_bound_state_only(&bound, number, "a checkbox")?;
                c = 5;
                Some((WKind::Checkbox { bound }, "a checkbox"))
            }
            "slider" => {
                let bound = self.bound_after(toks, 2, number, "a slider")?;
                self.check_bound_state_only(&bound, number, "a slider")?;
                c = 5;
                Some((
                    WKind::Slider {
                        bound,
                        from: 0.0,
                        to: 1.0,
                    },
                    "a slider",
                ))
            }
            "list" if is(2, "of") => {
                let list = word(3).unwrap_or_default().to_owned();
                self.check_list_ref(&list, number);
                c = 4;
                if is(c, "with") && is(c + 1, "varying") {
                    self.reserved(
                        number,
                        1,
                        "`a list of … with varying row heights`",
                        "v1 rows share one height; `each row N` sets it.",
                    );
                    return None;
                }
                Some((WKind::ListView { list, row: 48.0 }, "a list of"))
            }
            "empty" if is(2, "state") => {
                let Some(text) = str_at(3) else {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "an empty state carries its words: `an empty state \"No one here\"`",
                    );
                    return None;
                };
                let title = self.value_from_string(text, number)?;
                let (icon, next) = if is(4, "with") && is(5, "the") {
                    self.parse_icon(toks, 6, number)?
                } else {
                    (Icon::Close, 4)
                };
                c = next;
                Some((WKind::EmptyState { title, icon }, "an empty state"))
            }
            "icon" => {
                let (icon, next) = self.parse_icon(toks, 2, number)?;
                c = next;
                Some((WKind::IconOnly { icon }, "an icon"))
            }
            "floating" if is(2, "action") && is(3, "button") => {
                if !is(4, "with") || !is(5, "the") {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a floating action button names its icon: `a floating action button with the plus icon`",
                    );
                    return None;
                }
                let (icon, next) = self.parse_icon(toks, 6, number)?;
                c = next;
                Some((WKind::Fab { icon }, "a floating action button"))
            }
            _ => {
                let found = toks
                    .get(1)
                    .map_or_else(|| "nothing".to_owned(), Tok::describe);
                self.error(
                    number,
                    1,
                    codes::E060,
                    format!(
                        "no widget phrase starts with {found} — see the vocabulary in the Say docs"
                    ),
                );
                return None;
            }
        };
        let (kind, phrase) = result?;
        Some((kind, phrase, c))
    }

    /// `bound to <name>` — the two words after the phrase's head word.
    fn bound_after(&mut self, toks: &[Tok], c: usize, number: u32, what: &str) -> Option<String> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        if !(word(c) == Some("bound") && word(c + 1) == Some("to")) {
            self.error(
                number,
                1,
                codes::E060,
                format!("{what} binds to state: `{what} bound to s`"),
            );
            return None;
        }
        let bound = word(c + 2).unwrap_or_default().to_owned();
        self.check_bound(&bound, number)?;
        Some(bound)
    }

    /// A `bound to` target that must be plain state: a switch, a checkbox or
    /// a slider inside a dialog has nowhere to live — the dialog's value
    /// carries text fields only in v1.
    fn check_bound_state_only(&mut self, bound: &str, number: u32, what: &str) -> Option<()> {
        if self.inside_dialog > 0 {
            self.error(
                number,
                1,
                codes::E050,
                format!(
                    "{what} cannot live inside a dialog in this version — dialogs bind text fields"
                ),
            );
            return None;
        }
        self.check_bound(bound, number)
    }

    /// A `bound to` target: real state, or — inside a dialog block — a new
    /// field of the dialog's own value.
    fn check_bound(&mut self, bound: &str, number: u32) -> Option<()> {
        if bound.is_empty()
            || bound
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_ascii_lowercase())
            || !bound
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            self.error(
                number,
                1,
                codes::E031,
                format!("`{bound}` is not a name state can carry"),
            );
            return None;
        }
        if self.inside_dialog > 0 {
            // The dialog's value owns this field. It must not quietly shadow
            // a real state name — that way lies a dialog editing the wrong
            // thing.
            if self.kind_of(bound).is_some() {
                self.error(
                    number,
                    1,
                    codes::E031,
                    format!("a dialog field cannot share a name with the state `{bound}` — call it `dialog_{bound}`"),
                );
                return None;
            }
            if !self.modal_fields.iter().any(|f| f == bound) {
                self.modal_fields.push(bound.to_owned());
            }
        } else if self.kind_of(bound).is_none() {
            self.error(
                number,
                1,
                codes::E031,
                format!("nothing is called `{bound}` — known: {}", self.known_list()),
            );
            return None;
        }
        Some(())
    }

    fn check_list_ref(&mut self, list: &str, number: u32) {
        match self.kind_of(list) {
            Some(StateKind::ListText) | Some(StateKind::ListNumber) => {}
            _ => {
                self.error(
                    number,
                    1,
                    codes::E031,
                    format!("`{list}` is not a list — known: {}", self.known_list()),
                );
            }
        }
    }

    /// The icon words at `c`: `<name> icon` or `<direction> chevron`.
    fn parse_icon(&mut self, toks: &[Tok], c: usize, number: u32) -> Option<(Icon, usize)> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        let name = |i: usize| word(i).unwrap_or_default();
        let icon = match (name(c), name(c + 1)) {
            ("check", "icon") => Some((Icon::Check, c + 2)),
            ("close", "icon") => Some((Icon::Close, c + 2)),
            ("plus", "icon") => Some((Icon::Add, c + 2)),
            ("minus", "icon") => Some((Icon::Remove, c + 2)),
            ("left", "chevron") => Some((Icon::ChevronLeft, c + 2)),
            ("right", "chevron") => Some((Icon::ChevronRight, c + 2)),
            ("up", "chevron") => Some((Icon::ChevronUp, c + 2)),
            ("down", "chevron") => Some((Icon::ChevronDown, c + 2)),
            ("back", "chevron") => Some((Icon::ChevronBack, c + 2)),
            ("forward", "chevron") => Some((Icon::ChevronForward, c + 2)),
            _ => None,
        };
        match icon {
            Some(found) => Some(found),
            None => {
                self.error(
                    number,
                    1,
                    codes::E070,
                    "icons read `the check icon`, `the close icon`, `the plus icon`, `the minus icon` or `the <left/right/up/down> chevron`",
                );
                None
            }
        }
    }

    // -- properties --------------------------------------------------------

    fn parse_prop(
        &mut self,
        toks: &[Tok],
        c: &mut usize,
        number: u32,
        kind: &WKind,
        phrase: &str,
    ) -> Option<Prop> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        let str_at = |i: usize| -> Option<String> {
            toks.get(i).and_then(|t| match t {
                Tok::Str { text, .. } => Some(text.clone()),
                _ => None,
            })
        };
        let num_at = |i: usize| -> Option<f32> {
            toks.get(i).and_then(|t| match t {
                Tok::Float(f) => Some(*f),
                Tok::Int(i) => Some(*i as f32),
                _ => None,
            })
        };
        let int_at = |i: usize| -> Option<i64> {
            toks.get(i).and_then(|t| match t {
                Tok::Int(i) => Some(*i),
                Tok::Float(f) if f.fract() == 0.0 => Some(*f as i64),
                _ => None,
            })
        };

        let head = word(*c).unwrap_or_default().to_owned();
        let prop = match head.as_str() {
            "padded" => {
                let Some(n) = num_at(*c + 1) else {
                    self.error(
                        number,
                        1,
                        codes::E070,
                        "`padded` takes a number: `padded 16`",
                    );
                    return None;
                };
                *c += 2;
                Prop::Padded(n)
            }
            "spaced" => {
                let Some(n) = num_at(*c + 1) else {
                    self.error(
                        number,
                        1,
                        codes::E070,
                        "`spaced` takes a number: `spaced 12`",
                    );
                    return None;
                };
                *c += 2;
                Prop::Spaced(n)
            }
            "size" => {
                let Some(n) = num_at(*c + 1) else {
                    self.error(number, 1, codes::E070, "`size` takes a number: `size 14`");
                    return None;
                };
                *c += 2;
                Prop::Size(n)
            }
            "bold" => {
                *c += 1;
                Prop::Bold
            }
            "radius" => {
                let Some(n) = num_at(*c + 1) else {
                    self.error(
                        number,
                        1,
                        codes::E070,
                        "`radius` takes a number: `radius 12`",
                    );
                    return None;
                };
                *c += 2;
                Prop::Radius(n)
            }
            "color" => {
                let spec = self.color_at(toks, *c + 1, number)?;
                *c += 2;
                Prop::Color(spec)
            }
            "children" if word(*c + 1) == Some("aligned") => {
                let (align, next) =
                    self.align_words(toks, *c + 2, number, "`children aligned to the`")?;
                *c = next;
                Prop::ChildrenAligned(align)
            }
            "aligned" => {
                let (align, next) = self.align_words(toks, *c + 1, number, "`aligned`")?;
                *c = next;
                Prop::Aligned(match align {
                    Cross::Start => Align::Start,
                    Cross::Center => Align::Center,
                    Cross::End => Align::End,
                })
            }
            "from" => {
                let (Some(a), Some(b)) = (num_at(*c + 1), num_at(*c + 3)) else {
                    self.error(number, 1, codes::E070, "`from A to B` takes two numbers");
                    return None;
                };
                if word(*c + 2) != Some("to") {
                    self.error(number, 1, codes::E070, "a range reads `from 0.0 to 1.0`");
                    return None;
                }
                *c += 4;
                Prop::From { from: a, to: b }
            }
            "each" if word(*c + 1) == Some("row") => {
                let Some(n) = num_at(*c + 2) else {
                    self.error(
                        number,
                        1,
                        codes::E070,
                        "`each row` takes a number: `each row 56`",
                    );
                    return None;
                };
                *c += 3;
                Prop::EachRow(n)
            }
            "with" => {
                // with the <slot> color · with placeholder "…" ·
                // with varying row heights (reserved)
                if word(*c + 1) == Some("the") {
                    // Collect the words up to `color` — slots can be two
                    // words long (`on surface variant`).
                    let mut at = *c + 2;
                    let mut words: Vec<&str> = Vec::new();
                    while word(at).is_some() && word(at) != Some("color") {
                        words.push(word(at)?);
                        at += 1;
                    }
                    if word(at) == Some("color") {
                        let slot = self.theme_slot(words.join(" ").as_str(), number)?;
                        *c = at + 1;
                        Prop::Color(ColorSpec::Theme(slot))
                    } else {
                        self.error(
                            number,
                            1,
                            codes::E070,
                            "`with the …` reads `with the primary color` or `with placeholder \"…\"`",
                        );
                        return None;
                    }
                } else if word(*c + 1) == Some("placeholder") {
                    let Some(text) = str_at(*c + 2) else {
                        self.error(
                            number,
                            1,
                            codes::E070,
                            "`with placeholder` carries a quoted string",
                        );
                        return None;
                    };
                    *c += 3;
                    Prop::Placeholder(text)
                } else if word(*c + 1) == Some("varying") {
                    self.reserved(
                        number,
                        1,
                        "`a list of … with varying row heights`",
                        "v1 rows share one height; `each row N` sets it.",
                    );
                    return None;
                } else {
                    self.error(
                        number,
                        1,
                        codes::E070,
                        "a `with` clause reads `with the primary color` or `with placeholder \"…\"`",
                    );
                    return None;
                }
            }
            "single" if word(*c + 1) == Some("line") => {
                *c += 2;
                Prop::SingleLine
            }
            "in" if word(*c + 1) == Some("lines") => {
                // `in N lines` — the honest spelling of a taller field: the
                // field is given a height, because wrapping is always on.
                let Some(n) = int_at(*c + 2) else {
                    self.error(number, 1, codes::E070, "`in N lines` takes a whole number");
                    return None;
                };
                *c += 3;
                Prop::Lines(n.max(1) as u32)
            }
            "described" if word(*c + 1) == Some("as") => {
                let Some(text) = str_at(*c + 2) else {
                    self.error(
                        number,
                        1,
                        codes::E070,
                        "`described as` carries a quoted string",
                    );
                    return None;
                };
                *c += 3;
                Prop::Description(text)
            }
            "labelled" => {
                let Some(text) = str_at(*c + 1) else {
                    self.error(number, 1, codes::E070, "`labelled` carries a quoted string");
                    return None;
                };
                *c += 2;
                Prop::Labelled(text)
            }
            "pinned" => {
                let pin = match (word(*c + 1), num_at(*c + 2)) {
                    (Some("top"), Some(n)) => Pin::Top(n),
                    (Some("bottom"), Some(n)) => Pin::Bottom(n),
                    (Some("left"), Some(n)) => Pin::Left(n),
                    (Some("right"), Some(n)) => Pin::Right(n),
                    _ => {
                        self.error(
                            number,
                            1,
                            codes::E070,
                            "`pinned` reads `pinned top 12` — top, bottom, left or right",
                        );
                        return None;
                    }
                };
                *c += 3;
                Prop::Pin(pin)
            }
            _ if num_at(*c).is_some() => {
                // `<N> wide` · `<N> tall`
                let n = num_at(*c).unwrap_or(0.0);
                match word(*c + 1) {
                    Some("wide") => {
                        *c += 2;
                        Prop::Wide(n)
                    }
                    Some("tall") => {
                        *c += 2;
                        Prop::Tall(n)
                    }
                    other => {
                        self.error(
                            number,
                            1,
                            codes::E070,
                            format!(
                                "unexpected {} — properties read `padded 16`, `size 14`, `100 wide`, `only if …`",
                                other.map_or_else(|| "nothing".to_owned(), |w| format!("`{w}`"))
                            ),
                        );
                        return None;
                    }
                }
            }
            _ => {
                self.error(
                    number,
                    1,
                    codes::E070,
                    format!("`{head}` is not a property this line can carry"),
                );
                return None;
            }
        };

        // A handful of properties only make sense on the widget they shape.
        let applies = match (&prop, kind) {
            (Prop::Size(_) | Prop::Bold, WKind::Text { .. }) => true,
            (Prop::Size(_) | Prop::Bold, _) => false,
            (Prop::Placeholder(_) | Prop::SingleLine | Prop::Lines(_), WKind::TextField { .. }) => {
                true
            }
            (Prop::Placeholder(_) | Prop::SingleLine | Prop::Lines(_), _) => false,
            (Prop::EachRow(_), WKind::ListView { .. }) => true,
            (Prop::EachRow(_), _) => false,
            (Prop::Description(_), WKind::EmptyState { .. }) => true,
            (Prop::Description(_), _) => false,
            (Prop::Labelled(_), WKind::Fab { .. }) => true,
            (Prop::Labelled(_), _) => false,
            (Prop::ChildrenAligned(_), WKind::Column | WKind::Row) => true,
            (Prop::ChildrenAligned(_), _) => false,
            (Prop::Aligned(_), WKind::Card | WKind::Center) => true,
            (Prop::Aligned(_), _) => false,
            (Prop::Spaced(_), WKind::Column | WKind::Row) => true,
            (Prop::Spaced(_), _) => false,
            _ => true,
        };
        if applies {
            Some(prop)
        } else {
            let what = match &prop {
                Prop::Size(_) | Prop::Bold => "size",
                Prop::Placeholder(_) | Prop::SingleLine | Prop::Lines(_) => "a text field property",
                Prop::EachRow(_) => "each row",
                Prop::Description(_) => "described as",
                Prop::Labelled(_) => "labelled",
                Prop::ChildrenAligned(_) => "children aligned",
                Prop::Aligned(_) => "aligned",
                Prop::Spaced(_) => "spaced",
                _ => "this property",
            };
            self.error(
                number,
                1,
                codes::E070,
                format!("a {phrase} does not take `{what}`"),
            );
            None
        }
    }

    fn align_words(
        &mut self,
        toks: &[Tok],
        c: usize,
        number: u32,
        what: &str,
    ) -> Option<(Cross, usize)> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        let mut at = c;
        if word(at) == Some("to") {
            at += 1;
            if word(at) == Some("the") {
                at += 1;
            }
        }
        let align = match word(at) {
            Some("start") | Some("left") | Some("top") => Cross::Start,
            Some("center") | Some("middle") => Cross::Center,
            Some("end") | Some("right") | Some("bottom") => Cross::End,
            _ => {
                self.error(
                    number,
                    1,
                    codes::E070,
                    format!("{what} takes `start`, `center` or `end`"),
                );
                return None;
            }
        };
        Some((align, at + 1))
    }

    fn color_at(&mut self, toks: &[Tok], c: usize, number: u32) -> Option<ColorSpec> {
        match toks.get(c) {
            Some(Tok::Hex(value)) => Some(ColorSpec::Hex(*value)),
            Some(Tok::Word(w)) => {
                match w.as_str() {
                    "red" => Some(ColorSpec::Named(NamedColor::Red)),
                    "green" => Some(ColorSpec::Named(NamedColor::Green)),
                    "blue" => Some(ColorSpec::Named(NamedColor::Blue)),
                    "black" => Some(ColorSpec::Named(NamedColor::Black)),
                    "white" => Some(ColorSpec::Named(NamedColor::White)),
                    "transparent" => Some(ColorSpec::Named(NamedColor::Transparent)),
                    other => {
                        self.error(
                        number,
                        1,
                        codes::E070,
                        format!("`{other}` is not a colour — use `#RRGGBB` or `with the primary color`"),
                    );
                        None
                    }
                }
            }
            other => {
                self.error(
                    number,
                    1,
                    codes::E070,
                    format!(
                        "a colour reads `color #587AF6` — got {}",
                        other.map_or("nothing".to_owned(), Tok::describe)
                    ),
                );
                None
            }
        }
    }

    fn theme_slot(&mut self, words: &str, number: u32) -> Option<ThemeSlot> {
        match words {
            "primary" => Some(ThemeSlot::Primary),
            "on primary" => Some(ThemeSlot::OnPrimary),
            "surface" => Some(ThemeSlot::Surface),
            "on surface" => Some(ThemeSlot::OnSurface),
            "surface variant" => Some(ThemeSlot::SurfaceVariant),
            "on surface variant" => Some(ThemeSlot::OnSurfaceVariant),
            "outline" => Some(ThemeSlot::Outline),
            "error" => Some(ThemeSlot::Error),
            "on error" => Some(ThemeSlot::OnError),
            _ => {
                self.error(
                    number,
                    1,
                    codes::E070,
                    "theme colours read `with the primary color`, `with the surface color`, `with the error color`, …",
                );
                None
            }
        }
    }

    // -- actions -----------------------------------------------------------

    /// The lines of an event block at `indent`, from `start`.
    fn parse_actions_block(&mut self, start: usize, indent: u32) -> (Vec<ActionLine>, usize) {
        let mut actions = Vec::new();
        let mut i = start;
        while i < self.lines.len() {
            let (number, depth, words) = match self.lines.get(i) {
                Some(l) => (l.number, l.indent, crate::parse::line_words(l)),
                None => break,
            };
            let Some(depth) = depth else {
                i += 1;
                continue;
            };
            if depth < indent {
                break;
            }
            if depth > indent {
                self.error(
                    number,
                    1,
                    codes::E010,
                    "this line is indented further than the block it is in",
                );
                i += 1;
                continue;
            }
            if words.first().copied() == Some("rust") {
                self.reserved(
                    number,
                    1,
                    "a `rust:` block",
                    "v1 screens stay in Say; the escape hatch returns with the Say 1.1 update.",
                );
                i = self.skip_block(i + 1, indent + 1);
                continue;
            }
            let toks = self.lines[i].tokens.clone();
            match self.parse_action_line(&toks, number, indent, i) {
                Some((action, next)) => {
                    actions.push(action);
                    i = next;
                }
                None => i += 1,
            }
        }
        if actions.is_empty() {
            let near = self
                .lines
                .get(start.saturating_sub(1))
                .map_or(1, |l| l.number);
            self.warn(
                near,
                1,
                codes::W130,
                "this event block has no actions in it",
            );
        }
        (actions, i)
    }

    #[allow(clippy::too_many_lines)]
    fn parse_action_line(
        &mut self,
        toks: &[Tok],
        number: u32,
        indent: u32,
        index: usize,
    ) -> Option<(ActionLine, usize)> {
        let word = |c: usize| toks.get(c).and_then(Tok::word);
        let str_at = |c: usize| -> Option<String> {
            toks.get(c).and_then(|t| match t {
                Tok::Str { text, .. } => Some(text.clone()),
                _ => None,
            })
        };
        let mut c = 0usize;

        // `only if <cond>: <action>` — a guarded action on one line.
        let mut only_if = None;
        if word(c) == Some("only") && word(c + 1) == Some("if") {
            let (cond, next) = self.parse_cond(toks, c + 2, number)?;
            if toks.get(next) != Some(&Tok::Colon) {
                self.error(
                    number,
                    1,
                    codes::E070,
                    "a guarded action reads `only if <cond>: <action>`",
                );
                return None;
            }
            only_if = Some(cond);
            c = next + 1;
        }

        let head = word(c).unwrap_or_default().to_owned();
        let kind = match head.as_str() {
            "set" => {
                let target = word(c + 1)?.to_owned();
                if word(c + 2) != Some("to") {
                    self.error(number, 1, codes::E060, "`set` reads `set count to 0`");
                    return None;
                }
                let (value, _) = self.parse_expr(toks, c + 3, number, &[])?;
                self.check_set(&target, &value, number)?;
                AKind::Set { target, value }
            }
            "add" => {
                let (value, _) = self.parse_expr(toks, c + 1, number, &["to"])?;
                let at = find_stop(toks, c + 1, &["to"]);
                if word(at) != Some("to") {
                    self.error(number, 1, codes::E060, "`add` reads `add 1 to count`");
                    return None;
                }
                let target = word(at + 1)?.to_owned();
                self.check_arith(&target, &value, "add", number);
                AKind::Add { target, value }
            }
            "subtract" => {
                let (value, _) = self.parse_expr(toks, c + 1, number, &["from"])?;
                let at = find_stop(toks, c + 1, &["from"]);
                if word(at) != Some("from") {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "`subtract` reads `subtract 1 from count`",
                    );
                    return None;
                }
                let target = word(at + 1)?.to_owned();
                self.check_arith(&target, &value, "subtract", number);
                AKind::Sub { target, value }
            }
            "toggle" => {
                let target = word(c + 1)?.to_owned();
                self.check_target_kind(&target, Ty::Flag, "toggle", number);
                AKind::Toggle { target }
            }
            "append" => {
                let (value, _) = self.parse_expr(toks, c + 1, number, &["to"])?;
                let at = find_stop(toks, c + 1, &["to"]);
                if word(at) != Some("to") {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "`append` reads `append \"Ada\" to people`",
                    );
                    return None;
                }
                let target = word(at + 1)?.to_owned();
                self.check_append(&target, &value, number);
                AKind::Append { target, value }
            }
            "remove"
                if word(c + 1) == Some("the")
                    && word(c + 2) == Some("item")
                    && word(c + 3) == Some("at") =>
            {
                let (index_expr, _) = self.parse_expr(toks, c + 4, number, &["from"])?;
                let at = find_stop(toks, c + 4, &["from"]);
                if word(at) != Some("from") {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "`remove` reads `remove the item at 0 from people`",
                    );
                    return None;
                }
                let target = word(at + 1)?.to_owned();
                self.check_list_target(&target, number);
                self.check_expr(&index_expr, Some(Ty::Whole), number);
                AKind::RemoveAt {
                    target,
                    index: index_expr,
                }
            }
            "clear" => {
                let target = word(c + 1)?.to_owned();
                self.check_list_target(&target, number);
                AKind::Clear { target }
            }
            "open" if word(c + 1) == Some("the") && word(c + 2) == Some("screen") => {
                let Some(screen) = str_at(c + 3) else {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "name the screen: `open the screen \"About\"`",
                    );
                    return None;
                };
                self.pending_screen_refs.push((screen.clone(), number));
                AKind::Open { screen }
            }
            "open" if word(c + 1) == Some("a") && word(c + 2) == Some("dialog") => {
                let Some(title) = str_at(c + 4) else {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a dialog has a title: `open a dialog titled \"New person\":`",
                    );
                    return None;
                };
                if toks.get(c + 5) != Some(&Tok::Colon) {
                    self.error(number, 1, codes::E070, "a dialog block ends with `:`");
                    return None;
                }
                self.inside_dialog += 1;
                let (body, next) = self.parse_block(index + 1, indent + 1);
                self.inside_dialog -= 1;
                return Some((
                    ActionLine {
                        kind: AKind::OpenDialog { title, body },
                        only_if,
                        line: number,
                    },
                    next,
                ));
            }
            "go" if word(c + 1) == Some("back") => {
                self.has_go_back = true;
                AKind::GoBack
            }
            "show" if word(c + 1) == Some("a") && word(c + 2) == Some("snackbar") => {
                let Some(text) = str_at(c + 3) else {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a snackbar carries its message: `show a snackbar \"Saved\"`",
                    );
                    return None;
                };
                let msg = self.value_from_string(text, number)?;
                AKind::Snackbar { msg }
            }
            "close" if word(c + 1) == Some("this") && word(c + 2) == Some("dialog") => {
                if self.inside_dialog == 0 {
                    self.error(
                        number,
                        1,
                        codes::E090,
                        "`close this dialog` belongs inside a dialog block",
                    );
                    return None;
                }
                AKind::CloseDialog
            }
            "rust" => {
                self.reserved(
                    number,
                    1,
                    "a `rust:` block",
                    "v1 screens stay in Say; the escape hatch returns with the Say 1.1 update.",
                );
                return None;
            }
            _ => {
                self.error(
                    number,
                    1,
                    codes::E060,
                    "actions read `set count to 0`, `add 1 to count`, `toggle done`, `append \"Ada\" to people`, `open the screen \"About\"`, `go back`, `show a snackbar \"Saved\"`, …",
                );
                return None;
            }
        };
        Some((
            ActionLine {
                kind,
                only_if,
                line: number,
            },
            index + 1,
        ))
    }

    // -- conditions and expressions ----------------------------------------

    /// A condition, from `c`. Returns the condition and the cursor past it.
    /// Chains of `and` / `or` are left-associative and do not rank one above
    /// the other — a Say condition simple enough to need precedence is a
    /// condition that should be two.
    fn parse_cond(&mut self, toks: &[Tok], c: usize, number: u32) -> Option<(Cond, usize)> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        let mut cursor = c;

        let negated = word(cursor) == Some("not");
        if negated {
            cursor += 1;
        }
        let (first, mut next) = self.parse_cond_atom(toks, cursor, number)?;
        let mut cond = if negated {
            Cond::Not(Box::new(first))
        } else {
            first
        };

        loop {
            let joiner = word(next).unwrap_or_default().to_owned();
            if joiner != "and" && joiner != "or" {
                break;
            }
            let (rhs, after) = self.parse_cond_atom(toks, next + 1, number)?;
            cond = if joiner == "and" {
                Cond::And(Box::new(cond), Box::new(rhs))
            } else {
                Cond::Or(Box::new(cond), Box::new(rhs))
            };
            next = after;
        }
        Some((cond, next))
    }

    fn parse_cond_atom(&mut self, toks: &[Tok], c: usize, number: u32) -> Option<(Cond, usize)> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        let str_at = |i: usize| -> Option<String> {
            toks.get(i).and_then(|t| match t {
                Tok::Str { text, .. } => Some(text.clone()),
                _ => None,
            })
        };

        let Some(head) = word(c).map(str::to_owned) else {
            self.error(
                number,
                1,
                codes::E060,
                "a condition needs something to ask about",
            );
            return None;
        };

        // `how many people is 0` — `how many` opens the left side.
        let (lhs, mut next) = if head == "how" && word(c + 1) == Some("many") {
            let list = word(c + 2).unwrap_or_default().to_owned();
            self.check_expr(
                &Expr::Len {
                    list: list.clone(),
                    line: number,
                },
                None,
                number,
            );
            (Expr::Len { list, line: number }, c + 3)
        } else {
            let (expr, after) = self.parse_expr(toks, c, number, COND_STOPS)?;
            (expr, after)
        };

        // `name contains "Ada"` — no `is`.
        if word(next) == Some("contains") {
            let of = match &lhs {
                Expr::Ref { name, .. } => name.clone(),
                _ => {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "`contains` asks a piece of state: `query contains \"Ada\"`",
                    );
                    return None;
                }
            };
            let Some(what) = str_at(next + 1) else {
                self.error(number, 1, codes::E060, "`contains` takes a quoted string");
                return None;
            };
            let kind = self.kind_of(&of);
            if !matches!(kind, Some(StateKind::Text) | Some(StateKind::ListText)) {
                self.error(
                    number,
                    1,
                    codes::E04X,
                    format!("`contains` asks text or a list of text — `{of}` is neither"),
                );
                return None;
            }
            return Some((Cond::Contains { of, what }, next + 2));
        }

        if word(next) != Some("is") {
            // A bare flag: `only if subscribed`.
            match &lhs {
                Expr::Ref { name, .. } => match self.kind_of(name) {
                    Some(StateKind::Flag) => {
                        return Some((Cond::Flag { name: name.clone() }, next));
                    }
                    _ => {
                        self.error(
                            number,
                            1,
                            codes::E04X,
                            format!("a condition on its own must be a yes-or-no — `{name}` is not"),
                        );
                        return None;
                    }
                },
                _ => {
                    self.error(
                        number,
                        1,
                        codes::E060,
                        "a condition reads `count is 0` or `only if subscribed`",
                    );
                    return None;
                }
            }
        }
        next += 1;

        let mut at = next;
        let negated = if word(at) == Some("not") {
            at += 1;
            true
        } else {
            false
        };
        let (op, after_op) = match (word(at), word(at + 1)) {
            (Some("greater"), Some("than")) => (CmpOp::Gt, at + 2),
            (Some("less"), Some("than")) => (CmpOp::Lt, at + 2),
            (Some("at"), Some("least")) => (CmpOp::Ge, at + 2),
            (Some("at"), Some("most")) => (CmpOp::Le, at + 2),
            _ => (if negated { CmpOp::Ne } else { CmpOp::Eq }, at),
        };

        let (rhs, after) = self.parse_expr(toks, after_op, number, COND_STOPS)?;
        self.check_cmp(&lhs, op, &rhs, number);
        Some((Cond::Cmp { lhs, op, rhs }, after))
    }

    /// An expression from `c`, stopping before any word in `stops` (or at a
    /// comma/colon/end). `+` and `-` chain left-associatively; there is no
    /// precedence to explain, because there is no `*` yet to fight about.
    /// Always either succeeds or has reported a diagnostic.
    fn parse_expr(
        &mut self,
        toks: &[Tok],
        c: usize,
        number: u32,
        stops: &[&str],
    ) -> Option<(Expr, usize)> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        match toks.get(c) {
            None | Some(Tok::Comma) | Some(Tok::Colon) => {
                self.error(number, 1, codes::E060, "a value was expected here");
                return None;
            }
            Some(Tok::Word(w)) if stops.contains(&w.as_str()) => {
                self.error(number, 1, codes::E060, "a value was expected here");
                return None;
            }
            _ => {}
        }
        let mut cursor = c;
        let mut lhs = self.parse_term(toks, &mut cursor, number)?;
        loop {
            let op = match word(cursor) {
                Some("+") => BinOp::Add,
                Some("-") => BinOp::Sub,
                _ => break,
            };
            let mut after = cursor + 1;
            let rhs = self.parse_term(toks, &mut after, number)?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                line: number,
            };
            cursor = after;
        }
        Some((lhs, cursor))
    }

    /// Words an expression must never swallow: they are grammar, not values.
    const RESERVED_WORDS: &'static [&'static str] = &[
        "to", "from", "and", "or", "is", "not", "contains", "greater", "less", "than", "at",
        "least", "most", "when", "which", "the", "a", "an", "of", "in", "by",
    ];

    fn parse_term(&mut self, toks: &[Tok], cursor: &mut usize, number: u32) -> Option<Expr> {
        let word = |i: usize| toks.get(i).and_then(Tok::word);
        let c = *cursor;
        let expr = match toks.get(c) {
            Some(Tok::Int(v)) => {
                *cursor = c + 1;
                Expr::Whole(*v)
            }
            Some(Tok::Float(f)) => {
                *cursor = c + 1;
                Expr::Num(*f)
            }
            Some(Tok::Str { text, .. }) => {
                *cursor = c + 1;
                Expr::Str(text.clone())
            }
            Some(Tok::Word(w)) if w == "how" && word(c + 1) == Some("many") => {
                let list = word(c + 2).unwrap_or_default().to_owned();
                *cursor = c + 3;
                Expr::Len { list, line: number }
            }
            Some(Tok::Word(w)) if w == "yes" || w == "true" => {
                *cursor = c + 1;
                Expr::Flag(true)
            }
            Some(Tok::Word(w)) if w == "no" || w == "false" => {
                *cursor = c + 1;
                Expr::Flag(false)
            }
            Some(Tok::Word(w))
                if w.chars().next().is_some_and(|ch| ch.is_ascii_lowercase())
                    && w.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                    && !Self::RESERVED_WORDS.contains(&w.as_str()) =>
            {
                *cursor = c + 1;
                Expr::Ref {
                    name: w.clone(),
                    line: number,
                }
            }
            other => {
                self.error(
                    number,
                    1,
                    codes::E060,
                    format!(
                        "an expression is a number, text, a piece of state, or `how many <list>` — got {}",
                        other.map_or_else(|| "nothing".to_owned(), |t| t.describe())
                    ),
                );
                return None;
            }
        };
        Some(expr)
    }

    /// Split a string literal into literal parts and interpolated
    /// expressions. `\(count)` reads state; the whole rest is text.
    fn value_from_string(&mut self, text: String, number: u32) -> Option<Value> {
        if !text.contains("\\(") {
            return Some(Value::Lit(text));
        }
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' && chars.peek() == Some(&'(') {
                chars.next();
                // Read to the matching close paren, honouring nesting.
                let mut inner = String::new();
                let mut depth = 1usize;
                for inner_ch in chars.by_ref() {
                    if inner_ch == '(' {
                        depth += 1;
                    }
                    if inner_ch == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    inner.push(inner_ch);
                }
                let toks = tokenize(&inner, 0);
                let (expr, _) = self.parse_expr(&toks, 0, number, &[])?;
                parts.push(Part::Lit(std::mem::take(&mut literal)));
                parts.push(Part::Expr(expr));
            } else if ch == '\\' && chars.peek() == Some(&'\\') {
                chars.next();
                literal.push('\\');
            } else if ch == '\\' && chars.peek() == Some(&')') {
                chars.next();
                literal.push(')');
            } else {
                literal.push(ch);
            }
        }
        if !literal.is_empty() {
            parts.push(Part::Lit(literal));
        }
        Some(Value::Parts(parts))
    }

    // -- checks ------------------------------------------------------------

    fn check_set(&mut self, target: &str, value: &Expr, number: u32) -> Option<()> {
        if self.modal_fields.iter().any(|f| f == target) && self.inside_dialog > 0 {
            // Dialog fields are text values; `set draft to "…"` replaces the
            // whole value, which is the right semantics for a programmatic
            // write (the user is not holding the caret).
            self.check_expr(value, Some(Ty::Text), number);
            return Some(());
        }
        let Some(kind) = self.kind_of(target) else {
            self.error(
                number,
                1,
                codes::E031,
                format!(
                    "nothing is called `{target}` — known: {}",
                    self.known_list()
                ),
            );
            return None;
        };
        let ty = scalar(kind);
        match ty {
            Some(ty) => {
                self.check_expr(value, Some(ty), number);
                Some(())
            }
            None => {
                self.error(
                    number,
                    1,
                    codes::E04X,
                    format!("`set` replaces one value — use `append` or `clear` for the list `{target}`"),
                );
                None
            }
        }
    }

    fn check_arith(&mut self, target: &str, value: &Expr, verb: &str, number: u32) {
        let Some(kind) = self.kind_of(target) else {
            self.error(
                number,
                1,
                codes::E031,
                format!(
                    "nothing is called `{target}` — known: {}",
                    self.known_list()
                ),
            );
            return;
        };
        match kind {
            StateKind::Whole => self.check_expr(value, Some(Ty::Whole), number),
            StateKind::Number => self.check_expr(value, Some(Ty::Number), number),
            _ => {
                self.error(
                    number,
                    1,
                    codes::E04X,
                    format!("`{verb}` changes a number — `{target}` is not one"),
                );
            }
        }
    }

    fn check_append(&mut self, target: &str, value: &Expr, number: u32) {
        let Some(kind) = self.kind_of(target) else {
            self.error(
                number,
                1,
                codes::E031,
                format!(
                    "nothing is called `{target}` — known: {}",
                    self.known_list()
                ),
            );
            return;
        };
        match kind {
            StateKind::ListText => self.check_expr(value, Some(Ty::Text), number),
            StateKind::ListNumber => self.check_expr(value, Some(Ty::Number), number),
            _ => {
                self.error(
                    number,
                    1,
                    codes::E04X,
                    format!("`append` grows a list — `{target}` is not one"),
                );
            }
        }
    }

    fn check_list_target(&mut self, target: &str, number: u32) {
        match self.kind_of(target) {
            Some(StateKind::ListText) | Some(StateKind::ListNumber) => {}
            _ => {
                self.error(number, 1, codes::E04X, format!("`{target}` is not a list"));
            }
        }
    }

    fn check_target_kind(&mut self, target: &str, ty: Ty, verb: &str, number: u32) {
        match self.kind_of(target) {
            Some(kind) if scalar(kind) == Some(ty) => {}
            _ => {
                self.error(
                    number,
                    1,
                    codes::E04X,
                    format!("`{verb}` needs {a} — `{target}` is not", a = ty_name(ty)),
                );
            }
        }
    }

    fn check_cmp(&mut self, lhs: &Expr, op: CmpOp, rhs: &Expr, number: u32) {
        let lt = self.expr_ty(lhs, number);
        let rt = self.expr_ty(rhs, number);
        let (Some(lt), Some(rt)) = (lt, rt) else {
            return;
        };
        let ordered = matches!(op, CmpOp::Gt | CmpOp::Lt | CmpOp::Ge | CmpOp::Le);
        if lt != rt {
            self.error(
                number,
                1,
                codes::E04X,
                format!(
                    "a comparison asks the same kind on both sides — {} against {}",
                    ty_name(lt),
                    ty_name(rt)
                ),
            );
        } else if ordered && !matches!(lt, Ty::Whole | Ty::Number) {
            self.error(
                number,
                1,
                codes::E04X,
                format!(
                    "`greater than` and friends need numbers — this side is {}",
                    ty_name(lt)
                ),
            );
        }
    }

    /// Type an expression, reporting mismatches against `expect`.
    fn check_expr(&mut self, expr: &Expr, expect: Option<Ty>, number: u32) {
        let found = self.expr_ty(expr, number);
        let Some(found) = found else { return };
        if let Some(expect) = expect {
            if found != expect {
                self.error(
                    number,
                    1,
                    codes::E04X,
                    format!(
                        "this is {}, and here it must be {}",
                        ty_name(found),
                        ty_name(expect)
                    ),
                );
            }
        }
    }

    /// The type of an expression, checking references as it goes.
    fn expr_ty(&mut self, expr: &Expr, _number: u32) -> Option<Ty> {
        match expr {
            Expr::Whole(_) => Some(Ty::Whole),
            Expr::Num(_) => Some(Ty::Number),
            Expr::Str(_) => Some(Ty::Text),
            Expr::Flag(_) => Some(Ty::Flag),
            Expr::Ref { name, line } => match self.kind_of(name) {
                Some(kind) => scalar(kind).or_else(|| {
                    self.error(
                        *line,
                        1,
                        codes::E04X,
                        format!("`{name}` is a list — use `how many {name}`, or repeat over it"),
                    );
                    None
                }),
                None if self.modal_fields.iter().any(|f| f == name) => Some(Ty::Text),
                None => {
                    self.error(
                        *line,
                        1,
                        codes::E031,
                        format!("nothing is called `{name}` — known: {}", self.known_list()),
                    );
                    None
                }
            },
            Expr::Len { list, line } => {
                match self.kind_of(list) {
                    Some(StateKind::ListText) | Some(StateKind::ListNumber) => {}
                    _ => {
                        self.error(
                            *line,
                            1,
                            codes::E031,
                            format!("`how many` asks a list — `{list}` is not one"),
                        );
                    }
                }
                Some(Ty::Whole)
            }
            Expr::Bin { lhs, rhs, line, .. } => {
                let lt = self.expr_ty(lhs, _number);
                let rt = self.expr_ty(rhs, _number);
                match (lt, rt) {
                    (Some(Ty::Whole), Some(Ty::Whole)) => Some(Ty::Whole),
                    (Some(Ty::Number) | Some(Ty::Whole), Some(Ty::Number) | Some(Ty::Whole)) => {
                        Some(Ty::Number)
                    }
                    (Some(Ty::Text), Some(Ty::Text)) => {
                        self.error(
                            *line,
                            1,
                            codes::E04X,
                            "text joins in the string itself — \"Hello \\(name)\", not \"Hello\" + name",
                        );
                        None
                    }
                    _ => {
                        self.error(*line, 1, codes::E04X, "`+` and `-` work on numbers");
                        None
                    }
                }
            }
        }
    }
}

/// Words an expression must stop before, inside a condition.
const COND_STOPS: &[&str] = &[
    "and", "or", "is", "contains", "not", "greater", "less", "at",
];

/// The index of the first token from `start` that the value must stop
/// before — a stop word, punctuation, or the end of the line.
fn find_stop(toks: &[Tok], start: usize, stops: &[&str]) -> usize {
    let mut i = start;
    loop {
        match toks.get(i) {
            None => return i,
            Some(Tok::Comma) | Some(Tok::Colon) => return i,
            Some(Tok::Word(w)) if stops.contains(&w.as_str()) => return i,
            _ => i += 1,
        }
    }
}
