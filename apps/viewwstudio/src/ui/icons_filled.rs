//! Filled 24×24 icon paths, exactly as they were before the icon set was
//! redrawn as centrelines for stroking (see `ui::icons`).
//!
//! Kept as a separate module rather than folded back into `ui::icons`: that
//! module's paths are centrelines now and every other call site strokes them
//! through `chrome::glyph`. This one function-for-function subset restores the
//! old fill-based rendering for the activity bar specifically, without
//! touching what every other icon in the app draws with.

use vieww_foundation::{parse_path_data, IconData, Path};

fn icon(data: &str) -> IconData {
    parse_path_data(data).map_or_else(
        |_| {
            IconData::square24(Path::rect(vieww_foundation::Rect::new(
                4.0, 4.0, 20.0, 20.0,
            )))
        },
        IconData::square24,
    )
}

pub fn folder() -> IconData {
    icon("M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z")
}

pub fn search() -> IconData {
    icon(
        "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 \
         5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 \
         14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z",
    )
}

pub fn code() -> IconData {
    icon("M9.4 16.6 4.8 12l4.6-4.6L8 6l-6 6 6 6 1.4-1.4zm5.2 0 4.6-4.6-4.6-4.6L16 6l6 6-6 6-1.4-1.4z")
}

pub fn warning() -> IconData {
    icon("M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z")
}

pub fn dashboard() -> IconData {
    icon("M3 13h8V3H3v10zm0 8h8v-6H3v6zm10 0h8V11h-8v10zm0-18v6h8V3h-8z")
}

pub fn tune() -> IconData {
    icon(
        "M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 \
         4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z",
    )
}

pub fn swatch() -> IconData {
    icon(concat!("M3 3h8v8H3z", "M9 9h8v8H9z", "M6 14h5v5H6z",))
}

pub fn gear() -> IconData {
    icon(
        "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61 \
         l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24 \
         -.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22 \
         -.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94 \
         l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94 \
         l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 \
         1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 \
         15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z",
    )
}

pub fn book() -> IconData {
    icon("M21 5c-1.11-.35-2.33-.5-3.5-.5-1.95 0-4.05.4-5.5 1.5-1.45-1.1-3.55-1.5-5.5-1.5S2.45 4.9 1 6v14.65c0 .25.25.5.5.5.1 0 .15-.05.25-.05C3.1 20.45 5.05 20 6.5 20c1.95 0 4.05.4 5.5 1.5 1.35-.85 3.8-1.5 5.5-1.5 1.65 0 3.35.3 4.75 1.05.1.05.15.05.25.05.25 0 .5-.25.5-.5V6c-.6-.45-1.25-.75-2-1zm0 13.5c-1.1-.35-2.3-.5-3.5-.5-1.7 0-4.15.65-5.5 1.5V8c1.35-.85 3.8-1.5 5.5-1.5 1.2 0 2.4.15 3.5.5v11.5z")
}

pub fn branch() -> IconData {
    icon("M6 3a3 3 0 00-1 5.83v6.34a3 3 0 101.99 0V11h4a4 4 0 004-4V5.83A3 3 0 1013 5.83V7a2 2 0 01-2 2H6.99V8.83A3 3 0 006 3z")
}

pub fn export() -> IconData {
    icon("M12 2l4 4h-3v7h-2V6H8l4-4zM5 14h2v5h10v-5h2v5c0 1.1-.9 2-2 2H7c-1.1 0-2-.9-2-2v-5z")
}

pub fn lightbulb() -> IconData {
    icon("M9 21c0 .55.45 1 1 1h4c.55 0 1-.45 1-1v-1H9v1zm3-19C8.14 2 5 5.14 5 9c0 2.38 1.19 4.47 3 5.74V17c0 .55.45 1 1 1h6c.55 0 1-.45 1-1v-2.26c1.81-1.27 3-3.36 3-5.74 0-3.86-3.14-7-7-7z")
}
