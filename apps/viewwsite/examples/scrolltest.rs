//! Does a wheel notch move the page? Asked on the host, where it is a second.
//!
//! ```console
//! cargo run -p viewwsite --example scrolltest
//! ```
//!
//! The browser is the only place this page really runs, and the browser is the
//! worst place to find out that a wheel event went nowhere: the failure looks
//! exactly like a page that is not scrollable. `FrameDriver::handle_scroll` is
//! the same call `vieww-platform-web`'s wheel listener makes, so asking it here
//! answers the same question in a second and with a printable number.

use std::time::Duration;

use vieww::foundation::{Offset, ScrollEvent, Size};
use vieww::FrameDriver;

use viewwsite::{root, Os};

fn main() {
    let surface = Size::new(1280.0, 900.0);
    let mut driver = FrameDriver::new(surface);
    let tree = root(&mut driver, Some(Os::Linux));
    driver.set_root(tree);
    // The controller `root` made is inside the tree; this asks the driver what
    // the page did rather than holding a handle to it, which is the only thing
    // the browser can ask too.

    for _ in 0..4 {
        driver.draw_frame();
    }
    println!("after mount: {} elements", driver.elements().len());

    // A wheel notch downward, in the finger-equivalent convention: negative dy
    // is a finger travelling up the screen, which moves further into the page.
    let event = ScrollEvent::new(
        Offset::new(640.0, 450.0),
        Offset::new(0.0, -120.0),
        Duration::from_millis(16),
    );
    let handled = driver.handle_scroll(&event);
    for step in 1..=4 {
        driver.draw_frame_at(Duration::from_millis(16 * step));
    }
    println!("after one notch: handled {handled}");
}
