//! Tables, trees, accordions and the widgets that navigate them.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example data --release
//! ```
//!
//! # What this is for
//!
//! `docs/AIMS.md` §K banks a *Cheaper* claim on `DataTable`: it is a `ListView`
//! with a header, so it inherits virtualisation, and
//! `a_table_of_fifty_thousand_rows_does_not_build_fifty_thousand_rows` is the
//! counted version. That test proves the rows are not built. **It cannot prove
//! the table looks like a table**, and until this example nobody had seen one.
//!
//! The failure mode for a virtualised table is specific and invisible to a
//! test: the header and the body are laid out by different code, so a column
//! whose header is 120 wide and whose cells are 118 reads as a table with a
//! slight lean, and gets worse the further right you look.
//!
//! # Things to check
//!
//! 1. **Scroll the table.** The header stays; the rows move under it; **every
//!    cell stays under its own header** for the whole scroll. The row numbers
//!    stay consecutive — the same check `grid.rs` makes, and the one that
//!    catches a virtualisation window computed off by one.
//!
//! 2. **Press a sortable header.** The arrow flips and the rows reorder. The
//!    table does *not* sort the data — `on_sorted` reports and the application
//!    reorders, which is §K's design and is why the row count can stay a
//!    `usize`. Pressing a non-sortable header must do nothing at all.
//!
//! 3. **Expand the tree and the accordion.** Both change height; watch what is
//!    *below* them move by exactly that much and no more. An accordion that
//!    overshoots is animating to a measured height that included its own
//!    padding twice.
//!
//! 4. **Drag a chip onto a bin.** The target highlights while the payload is
//!    over it, and only for the kind it accepts — drag the blue chip onto the
//!    red bin and nothing should light up.
//!
//! 5. **The markdown block** is the widest thing here. Check the heading,
//!    emphasis, code span and list all read as different, and that the wrap
//!    point is the block's edge rather than the window's.
//!
//! # Run it in release
//!
//! Debug builds of the layout pass are ten to thirty times slower.

use std::collections::HashSet;
use std::rc::Rc;

use vieww_element::{DragController, Runtime, ScrollController, Signal};
use vieww_foundation::Size;
use vieww_gestures::ScrollPhysics;
use vieww_platform_winit::App;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, SortDirection};

const SURFACE: Size = Size {
    width: 960.0,
    height: 760.0,
};

const BACKGROUND: Color = ThemeData::dark().colors.surface;

/// How many rows the table has.
///
/// Larger than any screen on purpose: the point of the table is that this
/// number costs nothing until the rows are on screen, and a demo with twenty
/// rows would not be demonstrating it.
const ROWS: usize = 5_000;

fn main() {
    let app = App::new()
        .title("vieww — data")
        .size(SURFACE)
        .background(BACKGROUND);

    let result = app.run(move |driver: &mut FrameDriver| {
        let runtime = driver.elements().runtime().clone();
        driver.set_root(Shell {
            state: State::new(&runtime),
        });
    });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("data failed: {error}"),
    }
}

#[derive(Debug, Clone)]
struct State {
    sort: Signal<(usize, SortDirection)>,
    page: Signal<usize>,
    trail: Signal<usize>,
    expanded_tree: Signal<HashSet<String>>,
    expanded_panel: Signal<bool>,
    dropped: Signal<Option<String>>,
    drag: Rc<DragController>,
    /// The whole page's scroll position.
    scroll_page: ScrollController,
    /// The table's scroll position.
    ///
    /// A `ListView` reads its window from the `ScrollMetrics` a viewport
    /// publishes, so one outside a `Scrollable` has nothing to read: it falls
    /// back to a guessed dozen rows, keeps spacers as long as all five thousand,
    /// and — with no viewport there is also no clip — paints those rows straight
    /// over the pager underneath it. That is what this screen did until it was
    /// looked at.
    table: ScrollController,
    /// The feed's scroll position, and how many pages it has fetched.
    feed: ScrollController,
    pages: Signal<usize>,
}

impl State {
    fn new(runtime: &Runtime) -> Self {
        Self {
            sort: runtime.signal((0_usize, SortDirection::Ascending)),
            page: runtime.signal(0_usize),
            trail: runtime.signal(2_usize),
            expanded_tree: runtime.signal(HashSet::from(["src".to_owned()])),
            expanded_panel: runtime.signal(false),
            dropped: runtime.signal(None),
            drag: Rc::new(DragController::new(runtime)),
            scroll_page: ScrollController::new(runtime, ScrollPhysics::android()),
            table: ScrollController::new(runtime, ScrollPhysics::android()),
            feed: ScrollController::new(runtime, ScrollPhysics::android()),
            pages: runtime.signal(1_usize),
        }
    }
}

#[derive(Debug)]
struct Shell {
    state: State,
}

impl Widget for Shell {
    fn debug_name(&self) -> &'static str {
        "Shell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Theme::new(ThemeData::dark())
            .child(Body {
                state: self.state.clone(),
            })
            .into()
    }
}

widget_node_from!(Shell);

#[derive(Debug)]
struct Body {
    state: State,
}

impl Widget for Body {
    fn debug_name(&self) -> &'static str {
        "Body"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let trail = self.state.trail.get();
        let choose_trail = self.state.trail.clone();

        let columns =
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(32.0)
                .children(children![
                    Flexible::new(3).child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .spacing(16.0)
                            .children(children![
                                // Breadcrumbs above the thing they locate, which is
                                // the only placement anybody reads them in.
                                Breadcrumbs::new(
                                    ["Home", "Reports", "2026", "August"]
                                        .iter()
                                        .take(trail + 1)
                                        .map(|part| (*part).to_owned())
                                        .collect()
                                )
                                .on_selected(move |index| choose_trail.set(index)),
                                Constrained::new(Constraints::tight(Size::new(560.0, 300.0)))
                                    .child(Table {
                                        state: self.state.clone()
                                    }),
                                Pager {
                                    state: self.state.clone()
                                },
                            ])
                    ),
                    Flexible::new(2).child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .spacing(16.0)
                            .children(children![
                            Text::new("Tree").style(theme.text.title),
                            Tree {
                                state: self.state.clone()
                            },
                            Text::new("Accordion").style(theme.text.title),
                            Panel {
                                state: self.state.clone()
                            },
                            Text::new("Drag and drop").style(theme.text.title),
                            Bins {
                                state: self.state.clone()
                            },
                            Text::new("Infinite scroll").style(theme.text.title),
                            Constrained::new(Constraints::tight(Size::new(320.0, 150.0)))
                                .child(Feed {
                                    state: self.state.clone()
                                }),
                            Text::new("Markdown").style(theme.text.title),
                            Markdown::new(
                                "# Heading\n\nBody text with *emphasis*, `code` and a link.\n\n\
                                 - first item\n- second item\n- third item\n"
                            ),
                        ])
                    ),
                ]);

        // **The page scrolls.** The right-hand column alone is 922pt tall in
        // 712 of window, so without this the markdown block at the bottom
        // cannot be reached — and a `Flex` with nothing left to give places its
        // children anyway rather than clipping, so the only sign was a line on
        // stderr nobody reads.
        let scroll = self.state.scroll_page.clone();
        SafeArea::new()
            .child(
                Scrollable::vertical(scroll.offset())
                    .on_drag(scroll.on_drag())
                    .on_drag_end(scroll.on_drag_end())
                    .on_extents(scroll.on_extents())
                    .child(Padding::new(EdgeInsets::all(24.0)).child(columns)),
            )
            .into()
    }
}

widget_node_from!(Body);

/// The virtualised table. See the module docs for what to watch while scrolling.
#[derive(Debug)]
struct Table {
    state: State,
}

impl Widget for Table {
    fn debug_name(&self) -> &'static str {
        "Table"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let (column, direction) = self.state.sort.get();
        let sort = self.state.sort.clone();

        let scroll = self.state.table.clone();

        // **Inside a `Scrollable`, which is not decoration here.** A `ListView`
        // takes its window from the `ScrollMetrics` a viewport publishes; with
        // no viewport above it there is nothing to read, so it builds a guessed
        // dozen rows while keeping spacers as tall as all five thousand — and
        // the viewport is also what clips, so those spacers let the rows paint
        // straight over the pager below. Both are obvious the moment the screen
        // is looked at and neither is visible in a test.
        Scrollable::vertical(scroll.offset())
            .on_drag(scroll.on_drag())
            .on_drag_end(scroll.on_drag_end())
            .on_extents(scroll.on_extents())
            .child(
                DataTable::new(
                    vec![
                        DataColumn::new("#", 70.0),
                        // 190 rather than 200. The four widths are fixed and the
                        // column they sit in is three fifths of the page — 528pt on
                        // this window — so 530pt of columns overflowed it by two, every
                        // frame, in a row nothing clips.
                        DataColumn::new("Name", 190.0).sortable(),
                        DataColumn::new("Size", 120.0).sortable(),
                        DataColumn::new("Kind", 140.0),
                    ],
                    ROWS,
                    theme.metrics.touch_target,
                    // Called for the cells on screen only. `row` is an index, never a
                    // borrow into a `Vec` — nothing is allocated per row until it is
                    // visible, which is the property §K's counted test pins.
                    move |row: usize, column: usize| {
                        // The application owns the order. Descending is the same rows
                        // read backwards, which is the cheapest honest answer and enough
                        // to show that `on_sorted` reached something.
                        let index = match direction {
                            SortDirection::Ascending => row,
                            SortDirection::Descending => ROWS - 1 - row,
                        };
                        let theme = ThemeData::dark();
                        match column {
                            0 => Text::new(format!("{index}")).style(theme.text.label),
                            1 => Text::new(format!("record-{index:05}")).style(theme.text.body),
                            2 => {
                                Text::new(format!("{} kB", 4 + index % 96)).style(theme.text.label)
                            }
                            _ => Text::new(if index % 3 == 0 { "folder" } else { "file" })
                                .style(theme.text.label),
                        }
                        .into()
                    },
                )
                .sorted_by(column, direction)
                .on_sorted(move |column, direction| sort.set((column, direction))),
            )
            .into()
    }
}

widget_node_from!(Table);

#[derive(Debug)]
struct Pager {
    state: State,
}

impl Widget for Pager {
    fn debug_name(&self) -> &'static str {
        "Pager"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let page = self.state.page.get();
        let choose = self.state.page.clone();

        Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .spacing(16.0)
            .children(children![
                Pagination::new(page, 12).on_page_selected(move |next| choose.set(next)),
                Text::new(format!("page {} of 12", page + 1))
                    .style(theme.text.label)
                    .color(theme.colors.on_surface_variant),
            ])
            .into()
    }
}

widget_node_from!(Pager);

#[derive(Debug)]
struct Tree {
    state: State,
}

impl Widget for Tree {
    fn debug_name(&self) -> &'static str {
        "Tree"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let expanded = self.state.expanded_tree.get();
        let toggle = self.state.expanded_tree.clone();

        let leaf =
            |id: &str, label: &str| TreeNode::leaf(id, Text::new(label).style(theme.text.body));

        TreeView::new(
            vec![
                TreeNode::branch(
                    "src",
                    Text::new("src").style(theme.text.body),
                    vec![
                        leaf("src/lib.rs", "lib.rs"),
                        leaf("src/main.rs", "main.rs"),
                        TreeNode::branch(
                            "src/widgets",
                            Text::new("widgets").style(theme.text.body),
                            vec![leaf("src/widgets/text.rs", "text.rs")],
                        ),
                    ],
                ),
                TreeNode::branch(
                    "docs",
                    Text::new("docs").style(theme.text.body),
                    vec![leaf("docs/AIMS.md", "AIMS.md")],
                ),
            ],
            expanded.clone(),
        )
        .on_toggled(move |id| {
            let mut next = expanded.clone();
            if !next.remove(&id) {
                next.insert(id);
            }
            toggle.set(next);
        })
        .into()
    }
}

widget_node_from!(Tree);

#[derive(Debug)]
struct Panel {
    state: State,
}

impl Widget for Panel {
    fn debug_name(&self) -> &'static str {
        "Panel"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let open = self.state.expanded_panel.get();
        let toggle = self.state.expanded_panel.clone();

        Accordion::new(
            Text::new("Delivery options").style(theme.text.body),
            Padding::new(EdgeInsets::all(12.0)).child(
                Text::new("Standard delivery arrives in three to five days.")
                    .style(theme.text.label)
                    .color(theme.colors.on_surface_variant),
            ),
        )
        .expanded(open)
        .on_toggled(move |next| toggle.set(next))
        .into()
    }
}

widget_node_from!(Panel);

/// Two draggable chips and two bins, one of which refuses the other's payload.
#[derive(Debug)]
struct Bins {
    state: State,
}

/// What the blue bin takes. The type *is* the contract — `DragKind` is a
/// `TypeId`, so a target that accepts `Cool` cannot be handed a `Warm` by
/// mistake, and the refusal is not a runtime check anybody wrote.
#[derive(Debug)]
struct Cool;

/// What the red bin takes. See [`Cool`].
#[derive(Debug)]
struct Warm;

impl Widget for Bins {
    fn debug_name(&self) -> &'static str {
        "Bins"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let session: Rc<dyn vieww_widget::DragSession> = self.state.drag.clone();
        let dropped = self.state.dropped.get();

        let chip = |label: &'static str, cool: bool| {
            let payload: Rc<dyn std::any::Any> = if cool { Rc::new(Cool) } else { Rc::new(Warm) };
            let note = self.state.dropped.clone();
            Draggable::new(Rc::clone(&session), payload)
                .on_dropped(Rc::new(move |landed| {
                    if landed.is_some() {
                        note.set(Some(label.to_owned()));
                    }
                }))
                .child(
                    DecoratedBox::rounded(
                        if cool {
                            theme.colors.primary
                        } else {
                            theme.colors.error
                        },
                        theme.metrics.corner,
                    )
                    .child(
                        Padding::new(EdgeInsets::symmetric(8.0, 14.0)).child(
                            // **The `on_` colour that pairs with the fill.** The
                            // type scale's own colour is `on_surface`, which is
                            // light because the surface is dark — put on a light
                            // `primary` it is near-white on near-white, and the
                            // chip read as an empty rounded box until somebody
                            // looked at it. `ColorScheme` carries the pair for
                            // exactly this, and a filled anything has to use it.
                            Text::new(label).style(theme.text.label).color(if cool {
                                theme.colors.on_primary
                            } else {
                                theme.colors.on_error
                            }),
                        ),
                    ),
                )
        };

        let bin = |id: u64, label: &'static str, cool: bool| {
            DragTarget::new(
                Rc::clone(&session),
                id,
                if cool {
                    std::any::TypeId::of::<Cool>()
                } else {
                    std::any::TypeId::of::<Warm>()
                },
            )
            .builder(Rc::new(move |hovering| {
                // The highlight is the only feedback a drop target gives, so it
                // is the thing to look at: it must appear for an accepted kind
                // and stay absent for a refused one.
                let theme = ThemeData::dark();
                DecoratedBox::outlined(
                    if hovering {
                        theme.colors.primary
                    } else {
                        theme.colors.outline
                    },
                    if hovering { 3.0 } else { 1.0 },
                    theme.metrics.corner,
                )
                .child(
                    Constrained::new(Constraints::tight(Size::new(140.0, 72.0)))
                        .child(Center::new().child(Text::new(label).style(theme.text.label))),
                )
                .into()
            }))
        };

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .main_axis_size(MainAxisSize::Min)
            .spacing(12.0)
            .children(children![
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .spacing(12.0)
                    .children(children![chip("Cool", true), chip("Warm", false)]),
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .spacing(12.0)
                    .children(children![
                        bin(1, "Takes Cool", true),
                        bin(2, "Takes Warm", false)
                    ]),
                Text::new(match dropped {
                    Some(name) => format!("last drop: {name}"),
                    None => "nothing dropped yet".to_owned(),
                })
                .style(theme.text.label)
                .color(theme.colors.on_surface_variant),
            ])
            .into()
    }
}

widget_node_from!(Bins);

/// A list that fetches another page as its end comes into range.
///
/// The checklist's "infinite scroll automated trigger thresholds", and the whole
/// of it is `ScrollController::on_near_end`. Scroll to the bottom and the row
/// count grows; keep going and it grows again.
///
/// **Watch that it grows once per approach.** The threshold is edge-triggered,
/// so resting at the bottom must not keep fetching — a level-triggered version
/// would add a page on every frame, and the counter beside the list is how you
/// can tell which one you are looking at.
#[derive(Debug)]
struct Feed {
    state: State,
}

impl Widget for Feed {
    fn debug_name(&self) -> &'static str {
        "Feed"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let pages = self.state.pages.get();
        let rows = pages * 20;

        // Registered on every build, which is safe because it replaces rather
        // than accumulates — and it has to be re-registered, since the closure
        // closes over the page count it is about to increment.
        let grow = self.state.pages.clone();
        self.state.feed.on_near_end(120.0, move || {
            grow.set(grow.peek() + 1);
        });

        let scroll = self.state.feed.clone();
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .spacing(8.0)
            .children(children![
                Flexible::new(1).child(
                    Scrollable::vertical(scroll.offset())
                        .on_drag(scroll.on_drag())
                        .on_drag_end(scroll.on_drag_end())
                        .on_extents(scroll.on_extents())
                        .child(ListView::new(rows, 32.0, {
                            let theme = theme.clone();
                            Rc::new(move |row: usize| {
                                Text::new(format!("item {row}"))
                                    .style(theme.text.body)
                                    .into()
                            })
                        }))
                ),
                Text::new(format!("{pages} page(s), {rows} rows"))
                    .style(theme.text.label)
                    .color(theme.colors.on_surface_variant),
            ])
            .into()
    }
}

widget_node_from!(Feed);
