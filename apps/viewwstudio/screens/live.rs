// live.rs — vieww Studio's Live Preview reads this file.
//
// It is NOT compiled and not part of your app: the studio parses it and draws
// it, which is why it updates as you type and why it cannot run any logic. Use
// it to sketch a flow — the screens, what is on them, and the way between them.
// Press Render to see real code instead; that one compiles.
//
// Delete this file and the Live Preview turns off.

live! {
    screen "Home" {
        title "Inbox"
        row "Design Review" "3 new comments on the card layout"
        row "Sprint Planning" "5 tickets moved to In Progress"
        row "Release Notes" "Draft for v0.3 is ready to review"
        button "Open settings" -> "Settings"
    }

    screen "Detail" {
        title "Design Review"
        text "Rows, text, buttons, a switch and a counter are all this file can say by itself."
        // `mount` drops one of your own screens in, exactly as it last rendered.
        // Open that file and press Render once to fill this in.
        mount "card_grid.rs"
        button "Back" -> "Home"
    }

    screen "Settings" {
        title "Settings"
        toggle "Notifications"
        counter "Badge count"
        text "Changes here are a sketch, not state your app keeps."
        button "Done" -> "Home"
    }

    nav "Home" "Detail" "Settings"
}
