//! The caret: one field, blinking.
use std::time::Duration;
use vieww_animation::Ticker;
use vieww_element::Runtime;
use viewwstudio::caret::{Active, BLINK};
use viewwstudio::Studio;

fn studio() -> (Runtime, Studio) {
    let runtime = Runtime::new();
    (runtime.clone(), Studio::new(&runtime))
}

#[test]
fn exactly_one_field_shows_a_caret() {
    let (_r, studio) = studio();
    let all = [
        Active::Editor,
        Active::Find,
        Active::Replace,
        Active::Palette,
    ];

    for (open_find, open_replace, open_palette, expected) in [
        (false, false, false, Active::Editor),
        (true, false, false, Active::Find),
        (true, true, false, Active::Replace),
        (true, true, true, Active::Palette),
        (false, false, true, Active::Palette),
    ] {
        studio.find_open.set(open_find);
        studio.find_replacing.set(open_replace);
        studio.palette_open.set(open_palette);

        let showing: Vec<Active> = all
            .iter()
            .copied()
            .filter(|f| studio.caret_visible(*f))
            .collect();
        assert_eq!(
            showing,
            vec![expected],
            "four fields each painting a motionless caret is what this replaced"
        );
    }
}

#[test]
fn the_caret_blinks_and_typing_wakes_it() {
    let (_r, studio) = studio();
    assert!(studio.caret_visible(Active::Editor));

    studio.blink.borrow_mut().tick(Duration::from_secs(1));
    studio
        .blink
        .borrow_mut()
        .tick(Duration::from_secs(1) + BLINK);
    assert!(!studio.caret_visible(Active::Editor), "off phase");

    // A keystroke.
    let mut value = studio.active().unwrap().value;
    value.insert("x");
    studio.edit(value);
    assert!(
        studio.caret_visible(Active::Editor),
        "a keystroke that lands in the off phase reads as a dropped keystroke"
    );
}

#[test]
fn turning_the_blink_off_leaves_the_caret_on() {
    let (_r, studio) = studio();
    studio.blink_enabled.set(false);
    studio.blink.borrow_mut().tick(Duration::from_secs(1));
    studio
        .blink
        .borrow_mut()
        .tick(Duration::from_secs(1) + BLINK);
    assert!(
        studio.caret_visible(Active::Editor),
        "a blink somebody switched off must not take the caret with it"
    );
}
