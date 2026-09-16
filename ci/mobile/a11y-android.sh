#!/usr/bin/env bash
#
# Switch TalkBack on, then check what the semantics tree actually publishes.
#
#   ci/mobile/a11y-android.sh            # enable TalkBack, dump, report, leave it on
#   ci/mobile/a11y-android.sh --off      # turn TalkBack back off and exit
#   ci/mobile/a11y-android.sh --keep     # report without restoring anything
#   ci/mobile/a11y-android.sh --activate # watch for a TalkBack double-tap reaching the tree
#   ci/mobile/a11y-android.sh --modal    # open the drawer and check the screen behind goes quiet
#
# Needs adb, one device attached, and `dev.vieww.demo` already installed —
# this script never builds. Use `cargo apk run` for that.
#
# # What this can and cannot settle
#
# **`uiautomator` CAN see this tree, and this script asserted the opposite for
# weeks.** Corrected 2026-08-14, on a Redmi Note 7 Pro with TalkBack bound: the
# dump listed 23 nodes — `Tap me` as a Button, Day/Week/Month with exactly one
# `selected="true"`, RadioButtons carrying `checked`, ProgressBars carrying their
# values. That is the semantics tree, complete and correct, walked by the tool
# this file said could not walk it.
#
# **How the wrong claim survived.** The node counter here split the XML with
# `tr '>' '>\n'`, which does nothing — `tr` truncates SET2 to the length of
# SET1, so it mapped '>' to '>'. The count was 1 on every run regardless of
# content, and the note beneath it explained that 1 was expected because the
# nodes are virtual. A measurement that cannot move, paired with a reason why
# its fixed value is correct, reads exactly like a confirmed hypothesis.
#
# The underlying fact is still true — AccessKit publishes through an
# `AccessibilityNodeProvider` on the host view — and on 2026-08-13 a day was
# genuinely lost to reading a *placeholder* dump as a broken bridge. What was
# wrong was the conclusion drawn from it: that the dump is never evidence. Taken
# after the service has bound, it is the best evidence available here.
#
# So the verdict now comes from the dump when the dump has something to say, and
# falls back to the `a11y: published N node(s)` log line otherwise. **That line
# is not required**: `report_published_tree` only runs inside
# `Adapter::update_if_active`, so a tree served through `request_initial_tree`
# publishes without ever logging. Ears remain the check on the other side.
#
# # Why enabling TalkBack matters even for a dump
#
# Without an accessibility service connected, AccessKit's adapter sits in
# `State::Placeholder` and publishes a single `Role::Window` node — one
# featureless `android.view.View`, which looks exactly like the no-op adapter
# this bridge replaced. A dump taken without a screen reader running proves
# nothing either way. See `crates/vieww-platform-winit/src/a11y_android.rs`.

set -euo pipefail

package="dev.vieww.demo"
activity="android.app.NativeActivity"
# The user `am start` launches into. Named rather than left implicit because a
# device with a work or private profile installs per user, and every check below
# has to ask about the *same* user the launch will use.
user="0"
talkback="com.google.android.marvin.talkback/com.google.android.marvin.talkback.TalkBackService"
mode="report"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --off) mode="off" ;;
        --keep) mode="keep" ;;
        --activate) mode="activate" ;;
        --modal) mode="modal" ;;
        -h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "a11y-android: unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

command -v adb >/dev/null 2>&1 || { echo "a11y-android: adb not found." >&2; exit 127; }
adb shell true >/dev/null 2>&1 || {
    echo "a11y-android: no device reachable." >&2
    adb devices -l >&2
    exit 3
}

disable_talkback() {
    adb shell settings put secure accessibility_enabled 0 >/dev/null 2>&1 || true
    adb shell settings delete secure enabled_accessibility_services >/dev/null 2>&1 || true
}

if [ "$mode" = "off" ]; then
    disable_talkback
    echo "a11y-android: TalkBack disabled."
    exit 0
fi

# Scoped to `$user`, and exactly, on a rule this repository has earned three
# times over: a check that cannot see the thing under test returns a clean,
# confident, wrong answer.
#
# `pm list packages` unscoped reports every user on the device. On a phone with
# a private or work profile that answers "installed" for a package `am start`
# then cannot find, and the two messages are each correct about a different
# user — which reads as a broken activity name rather than a missing install.
if ! adb shell pm list packages --user "$user" 2>/dev/null |
    grep -qx "package:$package"; then

    # Distinguish "never built" from "installed somewhere else", because the
    # fixes are a toolchain away from each other: one needs a build, the other
    # is already on the device and needs one line.
    other=$(adb shell pm list users 2>/dev/null |
        sed -n 's/.*UserInfo{\([0-9]*\):.*/\1/p' |
        while read -r candidate; do
            [ "$candidate" = "$user" ] && continue
            if adb shell pm list packages --user "$candidate" 2>/dev/null |
                grep -qx "package:$package"; then
                echo "$candidate"
            fi
        done | head -1)

    if [ -n "$other" ]; then
        echo "a11y-android: $package is installed for user $other, not user $user." >&2
        echo "  The apk is already on the device, so this needs no build:" >&2
        echo "    adb shell pm install-existing --user $user $package" >&2
    else
        echo "a11y-android: $package is not installed for user $user. Run:" >&2
        # Both lines, and in this order. `cargo apk` reads the SDK from the
        # environment, so the bare command fails in a fresh shell with "Android
        # SDK is not found" — which is a resolvable setup step wearing the
        # costume of a broken toolchain. `ci/mobile/android-env.sh` is the one place
        # that knows where the SDK and a *complete* NDK are; three scripts
        # rediscovered that separately before it existed.
        echo "  . ci/mobile/android-env.sh && android_env \"a11y\"" >&2
        echo "  cargo apk run -p vieww-platform-winit --example android --release" >&2
    fi
    exit 4
fi

# TalkBack has to be running *before* the tree is asked for, or the adapter is
# still holding its placeholder when the dump lands.
if adb shell pm list packages | grep -q "com.google.android.marvin.talkback"; then
    adb shell settings put secure enabled_accessibility_services "$talkback" >/dev/null
    adb shell settings put secure accessibility_enabled 1 >/dev/null
    echo "a11y-android: TalkBack enabled."
else
    echo "a11y-android: TalkBack is not installed on this device." >&2
    echo "  The dump below will be the placeholder, which proves nothing." >&2
    echo "  Install TalkBack (Android Accessibility Suite) and run this again." >&2
fi

adb shell am force-stop --user "$user" "$package" >/dev/null 2>&1 || true
adb shell am start --user "$user" -n "$package/$activity" >/dev/null
# The adapter answers `request_initial_tree` with `None` and posts to the event
# loop, so the real tree arrives a frame later. Asking too early gets the
# placeholder and reads as a failure.
sleep 3

# TalkBack opens a "Welcome to TalkBack" tutorial the first time it is enabled,
# and that tutorial takes the foreground *and* accessibility focus. A dump taken
# then describes TalkBack's own screen and looks exactly like our tree being
# empty — an hour lost to reading the wrong window.
top="$(adb shell dumpsys accessibility 2>/dev/null | grep -A1 'Top Focused Window Id' | tr -d '\r' || true)"
front="$(adb shell dumpsys activity activities 2>/dev/null | grep 'topResumedActivity' | tr -d '\r' || true)"
case "$front" in
    *"$package"*) ;;
    *)
        echo "a11y-android: $package is NOT the foreground app." >&2
        echo "  front: $front" >&2
        echo "  If this is TalkBack's tutorial, dismiss it (back a few times)" >&2
        echo "  and run this again — a dump now would describe the wrong window." >&2
        exit 5
        ;;
esac

pid="$(adb shell pidof -s "$package" 2>/dev/null | tr -d '\r' || true)"

# **Wait before printing anything, and the order is the whole point.**
#
# `|| true` first: `set -o pipefail` plus `set -e` makes a `grep` that matches
# nothing fatal, and the no-match case here *is* the failure this script exists
# to report. On 2026-08-14 that killed the run between the dump and the verdict,
# so the output ended mid-report and read like a clean finish. A diagnostic that
# dies on the diagnosis is worse than no diagnostic — the same note is on
# `running_pid` in `device-suite.sh`.
#
# Then the wait. `Adapter::update_if_active` runs its closure only once a
# service has *bound*, and TalkBack takes several seconds to bind to a freshly
# started activity — longer than the `sleep 3` above. Sampling once reports a
# working bridge as a dead one.
#
# And this runs **before** the log is printed, which the first fixed version got
# wrong: the evidence section was dumped at three seconds, the verdict was
# decided at thirteen, and the run that followed showed a report with no
# `a11y: published` line under a verdict saying the tree was published. Both
# halves were true and the pair was unreadable. Whatever the verdict is drawn
# from, the reader has to be looking at the same thing.
published=""
for _ in $(seq 10); do
    published="$(adb logcat -d --pid="${pid:-0}" 2>/dev/null | grep "a11y: published" | tail -1 || true)"
    [ -n "$published" ] && break
    sleep 1
done

# Separate evidence, and it answers a different question. `published` is absent
# whenever the tree was served through `request_initial_tree`, which never logs
# — the script says so itself a few lines down — so an empty `published` cannot
# distinguish "the bridge is not there" from "the bridge answered early and
# quietly". The injection line does: it is written when the delegate is put into
# the activity's view, whatever happens afterwards.
injected="$(adb logcat -d --pid="${pid:-0}" 2>/dev/null | grep -c "a11y: injected" || true)"

echo
echo "--- a11y log lines -------------------------------------------------------"
if [ -n "$pid" ]; then
    adb logcat -d --pid="$pid" 2>/dev/null | grep "a11y" || echo "(none — the adapter was never constructed)"
else
    echo "(the app is not running)"
fi

echo
echo "--- accessibility tree ---------------------------------------------------"
# Polled until the app's own nodes appear, because **this is what made the tree
# look invisible for months**. The script force-stops, restarts and dumps about
# thirteen seconds later; this app publishes its semantics tree at roughly
# thirty-five, after vello has compiled its shaders. So the dump landed on an
# activity that had drawn nothing, came back with six chrome nodes and no
# labels, and the conclusion drawn from it — that uiautomator cannot walk
# AccessKit's virtual nodes — was written into this script as a platform fact.
#
# It is not one. Given time, the dump shows every Button, EditText, RadioButton
# and ProgressBar with real bounds. Measured 2026-08-15, once the 0x0 host view
# was fixed: 6 nodes at 13s, 28 nodes at 60s, same build and same device.
dump=""
for _ in $(seq 60); do
    dump="$(adb exec-out uiautomator dump /dev/tty 2>/dev/null | tr -d '\r')"
    printf '%s' "$dump" | sed 's/></>\n</g' | grep "package=\"$package\"" |
        grep -q 'content-desc="[^"]\+"\|text="[^"]\+"' && break
    sleep 1
done
# One node per line. **`tr '>' '>\n'` was here and did nothing** — `tr` truncates
# SET2 to the length of SET1, so it mapped '>' to '>' and left the XML as a
# single line. Every count below was therefore 1/0/0 on every run this script has
# ever done, and the note underneath explained why 1 was the expected answer. A
# measurement that cannot move, with a reassuring explanation attached, is how a
# false premise survives for weeks. See the header.
nodes="$(printf '%s' "$dump" | sed 's/></>\n</g')"
printf '%s\n' "$nodes" | grep "$package" || true

echo
echo "--- what that means ------------------------------------------------------"
ours="$(printf '%s\n' "$nodes" | grep -c "package=\"$package\"" || true)"
labelled="$(printf '%s\n' "$nodes" | grep "package=\"$package\"" | grep -c 'content-desc="[^"]\+"' || true)"
texted="$(printf '%s\n' "$nodes" | grep "package=\"$package\"" | grep -c 'text="[^"]\+"' || true)"

# Content, not chrome. **Branching on `$ours` was wrong**, and wrong in the way
# this repository keeps rediscovering: every Android window carries a
# FrameLayout, a LinearLayout, an `android:id/content`, the host view and two
# system-bar backgrounds, all tagged with the app's package. That is six nodes
# on any app whatsoever, so `ours > 2` was true before the adapter did anything
# — and the script used it to announce that AccessKit's virtual nodes are
# visible to uiautomator, overturning its own earlier and correct claim.
#
# `labelled` and `texted` were being computed and printed correctly the whole
# time. The instrument had the right number in its hand and tested a different
# one.
described=$((labelled + texted))

echo "nodes belonging to $package : $ours  (six of these are window chrome)"
echo "  ... carrying a content-desc : $labelled"
echo "  ... carrying text           : $texted"
echo
if [ "$described" -gt 0 ]; then
    echo "VERDICT: the tree is published, and uiautomator can see it."
    echo
    echo "  $described node(s) carry a label or text, which no amount of window"
    echo "  chrome would produce."
    echo
    echo "  What is left needs ears. Swipe right through the screen and ask:"
    echo "    1. is every control reached?          (a skipped one is invisible)"
    echo "    2. does each say what it *is*?        (\"button\", \"checked\" — not"
    echo "                                           only its label)"
    echo "    3. does double-tap activate the right thing?"
    if [ "$mode" != "activate" ]; then
        echo
        echo "  For the third, run: ci/mobile/a11y-android.sh --activate"
    fi
elif [ "${injected:-0}" -gt 0 ]; then
    echo "VERDICT: the bridge is installed, but nothing of the app is in the dump."
    echo
    echo "  $ours nodes and not one label or text among them: everything here is"
    echo "  window chrome. **This is a fault, not a platform limitation**, and"
    echo "  this script asserted the opposite for months on the strength of it."
    echo
    echo "  Given a laid-out app and a host view with real bounds, the dump shows"
    echo "  every control. Two things are known to produce this instead:"
    echo "    - the dump was taken before the first frame (the loop above waits"
    echo "      60s; a slower start than that would still land here);"
    echo "    - the delegate is hosted on a zero-sized view, which is invisible"
    echo "      to hit testing — see a11y_android.rs::host_view."
    echo
    echo "  Ears remain the check for what the tree *says*. Swipe right:"
    echo "    1. is every control reached?          (a skipped one is invisible)"
    echo "    2. does each say what it *is*?        (\"button\", \"checked\" — not"
    echo "                                           only its label)"
    echo "    3. does double-tap activate the right thing?"
    echo
    echo "  Confirmed by ear on a Redmi Note 7 Pro, 2026-08-15: labels, button"
    echo "  names and double-tap activation all correct."
elif ! echo "$published" | grep -q "published"; then
    echo "VERDICT: nothing published — no screen reader ever activated the adapter."
    echo
    # The distinction that decides where to look, and it is not guessable from
    # this side: `Adapter::update_if_active` runs its closure only once a
    # service has *bound*. No binding means the fault is on the Android side —
    # a setting that did not take, or TalkBack sitting in its tutorial — and
    # nothing in this repository will change it.
    bound="$(adb shell dumpsys accessibility 2>/dev/null | grep -ci "talkback" || true)"
    running="$(adb shell pidof com.google.android.marvin.talkback 2>/dev/null | tr -d '\r' || true)"
    if [ -z "$running" ]; then
        echo "  TalkBack is NOT RUNNING (no process). The settings were written and"
        echo "  did not take — common on MIUI, which gates accessibility services"
        echo "  behind its own permission screen."
        echo "  Turn it on by hand: Settings > Accessibility > TalkBack, then"
        echo "  re-run. This is an Android-side problem, not a vieww one."
    else
        echo "  TalkBack IS running (pid $running, $bound accessibility mentions),"
        echo "  so it never bound to this window. Two usual causes:"
        echo "    - it is still in its welcome tutorial, holding focus itself;"
        echo "    - the activity was restarted underneath it and it did not rebind."
        echo "  Dismiss anything TalkBack is showing, then re-run."
    fi
    echo
    echo "  Either way the a11y injection lines above show the bridge installed"
    echo "  correctly; this is about whether anything asked it for a tree."
elif echo "$published" | grep -q "declined"; then
    echo "VERDICT: tree_update declined — the semantics tree had no root."
else
    echo "VERDICT: the tree is being published. What is left needs ears."
    echo
    echo "  What is left needs ears, and only ears. Swipe right through the whole"
    echo "  screen with one finger and ask, in this order:"
    echo "    1. is every control reached?          (a skipped one is invisible)"
    echo "    2. does each say what it *is*?        (\"button\", \"checked\" — not"
    echo "                                           only its label)"
    echo "    3. does double-tap activate the right thing?"
    echo
    echo "  The third is the one nothing above can check: it exercises"
    echo "  FrameDriver::handle_semantic_action end to end."
fi

# The fourth accessibility question, and the only one a script can settle.
#
# The other three are about what TalkBack *says*, and ears are the only
# instrument for that. This one is about what the app *receives*, and the app can
# be watched: `App::access_action` logs every action the bridge delivers, so a
# real double-tap leaves a line whether or not anything handled it.
#
# **Why the tap is not fully automated.** With explore-by-touch on, TalkBack
# consumes the touch stream: one tap moves accessibility focus and a double-tap
# activates whatever holds it. `adb shell input tap` synthesises taps that
# TalkBack does see, but the two have to land inside its double-tap window, and
# that window is a user setting on a device this script does not own. So it
# tries, and then asks — and either way the verdict comes from the log rather
# than from anybody's impression.
if [ "$mode" = "activate" ]; then
    echo
    echo "--- double-tap activation ------------------------------------------------"

    # Polled, not sampled once. The app reports its tap targets from the first
    # laid-out frame, and on an Adreno 612 in a debug build that is **thirty-odd
    # seconds** after `am start` — vello compiles its shader modules first, which
    # the log says out loud. Everything above this point takes about thirteen
    # seconds, so a single grep here looked before the line could exist and
    # reported the position as unknown on a run that was working perfectly.
    #
    # Measured 2026-08-15: process start 21:32:29, `__VIEWW_TAPS__` at 21:33:04.
    coords=""
    for _ in $(seq 45); do
        coords="$(adb logcat -d --pid="${pid:-0}" 2>/dev/null |
            grep "__VIEWW_TAPS__" | tail -1 || true)"
        [ -n "$coords" ] && break
        sleep 1
    done
    button="$(echo "$coords" | sed -n 's/.*button=\([0-9]*\),\([0-9]*\).*/\1 \2/p')"
    if [ -z "$button" ]; then
        echo "No __VIEWW_TAPS__ line after 45s, so the button's position is" >&2
        echo "  unknown. The app reports it from the first laid-out frame, so" >&2
        echo "  either it never got there or it crashed on the way. The a11y log" >&2
        echo "  lines above say which." >&2
        exit 6
    fi
    set -- $button
    bx="$1"; by="$2"
    echo "button at ${bx},${by} (physical px, from the semantics tree)"

    # Everything from here is new, so a line from an earlier run cannot be read
    # as this attempt succeeding.
    adb logcat -c >/dev/null 2>&1 || true

    # Focus, then activate. Two separate taps rather than a gesture: TalkBack's
    # own activation is a double-tap anywhere once something holds focus.
    adb shell input tap "$bx" "$by" >/dev/null 2>&1 || true
    sleep 1
    adb shell input tap "$bx" "$by" >/dev/null 2>&1 || true
    adb shell input tap "$bx" "$by" >/dev/null 2>&1 || true
    sleep 2

    delivered="$(adb logcat -d --pid="${pid:-0}" 2>/dev/null | grep "a11y:.*delivered" | tail -1 || true)"
    if [ -z "$delivered" ]; then
        echo
        echo "Nothing delivered by the synthesised taps — expected on many devices."
        echo "**Do it by hand now:** single-tap the button to focus it, then"
        echo "double-tap anywhere. Waiting 30 seconds..."
        for _ in $(seq 30); do
            delivered="$(adb logcat -d --pid="${pid:-0}" 2>/dev/null | grep "a11y:.*delivered" | tail -1 || true)"
            [ -n "$delivered" ] && break
            sleep 1
        done
    fi

    echo
    if [ -z "$delivered" ]; then
        echo "VERDICT: no action observed — and this run cannot say why."
        echo
        echo "  Two things produce this line and **the script cannot tell them"
        echo "  apart**: the bridge dropping TalkBack's activation, or nobody"
        echo "  having double-tapped inside the thirty seconds above. The"
        echo "  synthesised taps are expected to deliver nothing on many"
        echo "  devices, so on an unattended run this is the normal outcome and"
        echo "  is evidence of nothing at all."
        echo
        echo "  It is a fault **only if a double-tap actually happened** while it"
        echo "  was waiting. If one did, the suspect is accesskit_android rather"
        echo "  than FrameDriver: the in-app check"
        echo "  a_semantic_click_activates_the_button covers the routing and"
        echo "  passes independently."
        echo
        echo "  Confirmed by ear on a Redmi Note 7 Pro, 2026-08-15: a TalkBack"
        echo "  double-tap does activate the right control. This branch has not"
        echo "  been seen to mean anything else."
    elif echo "$delivered" | grep -q "handled=true"; then
        echo "VERDICT: delivered AND handled — the fourth question is answered."
        echo "  $delivered"
        echo "  A TalkBack double-tap reached FrameDriver::handle_semantic_action"
        echo "  and a render object carried it out."
    else
        echo "VERDICT: delivered but NOT handled — a routing bug, not a bridge one."
        echo "  $delivered"
        echo "  The action arrived and nothing in the subtree would take it."
    fi
fi

# The modal question: with a panel up, does the screen behind it leave the tree?
#
# # Why this is a *second* instrument and not a duplicate of the in-app check
#
# `a_modal_silences_the_screen_behind_it` in `shared/device_tests.rs` asks
# vieww's own `SemanticsTree` and runs on every device run with nobody watching.
# It proves the framework builds a blocked tree. It cannot prove that the
# **bridge publishes** the blocked tree — `accesskit_android` could be serving a
# cached update, or the block could be lost crossing into
# `AccessibilityNodeProvider`, and the in-app check would pass through both.
#
# This asks uiautomator, which stands where TalkBack stands. Together they
# separate "vieww got it wrong" from "the bridge got it wrong", and one
# observation cannot.
#
# # The labels come out of the source, not out of this file
#
# `TAP_LABEL` and `MENU_ITEMS` are defined once in `shared/screen.rs`, with a
# comment saying why a second copy is a trap: a check carrying its own copy of a
# string silently stops finding anything the moment somebody rewords a control,
# and reports that as the app being broken. `ci/mobile/reload-watch-android-test.sh`
# lifts the functions it tests out of the real script for the same reason. So
# these are parsed out of the Rust, and a parse that finds nothing is a hard
# failure rather than an empty search that would pass by matching nothing.
if [ "$mode" = "modal" ]; then
    echo
    echo "--- modal semantics ------------------------------------------------------"

    root="$(cd "$(dirname "$0")/../.." && pwd)"
    screen="$root/crates/vieww-platform-winit/examples/shared/screen.rs"
    [ -f "$screen" ] || {
        echo "a11y-android: cannot find $screen" >&2
        exit 7
    }

    tap_label="$(sed -n 's/^pub(crate) const TAP_LABEL: &str = "\(.*\)";$/\1/p' "$screen")"
    # One item per line, from the array literal.
    menu_items="$(sed -n 's/^pub(crate) const MENU_ITEMS: \[&str; [0-9]*\] = \[\(.*\)\];$/\1/p' "$screen" |
        tr ',' '\n' | sed 's/^[[:space:]]*"//; s/"[[:space:]]*$//' | grep -v '^$')"

    if [ -z "$tap_label" ] || [ -z "$menu_items" ]; then
        echo "a11y-android: could not parse TAP_LABEL or MENU_ITEMS out of" >&2
        echo "  $screen" >&2
        echo "  Those constants moved or were reformatted. Fix the two sed" >&2
        echo "  expressions above rather than pasting the strings in here —" >&2
        echo "  a copy is what this parsing exists to avoid." >&2
        exit 7
    fi
    echo "from screen.rs: behind=\"$tap_label\"  panel=$(echo "$menu_items" | tr '\n' ' ')"

    # Same polling as `--activate`, and for the same reason: the app reports its
    # targets from the first laid-out frame, which is thirty-odd seconds after
    # `am start` on an Adreno 612 because vello compiles shaders first.
    coords=""
    for _ in $(seq 45); do
        coords="$(adb logcat -d --pid="${pid:-0}" 2>/dev/null |
            grep "__VIEWW_TAPS__" | tail -1 || true)"
        [ -n "$coords" ] && break
        sleep 1
    done
    menu="$(echo "$coords" | sed -n 's/.*menu=\([0-9]*\),\([0-9]*\).*/\1 \2/p')"
    if [ -z "$menu" ]; then
        echo "No menu= in __VIEWW_TAPS__ after 45s." >&2
        echo "  The app reports that token only when the drawer's trigger is in" >&2
        echo "  the semantics tree. Either the frame never arrived — the a11y" >&2
        echo "  lines above say — or the button laid out past the bottom of this" >&2
        echo "  screen, which on a short device is a real possibility and is why" >&2
        echo "  the token is optional. Check the screen." >&2
        exit 7
    fi
    set -- $menu
    mx="$1"; my="$2"
    echo "menu button at ${mx},${my} (physical px, from the semantics tree)"

    # Focus, then activate. With explore-by-touch on, one tap moves accessibility
    # focus and a double-tap presses — the same dance `--activate` does, and for
    # the same reason: TalkBack owns the touch stream while it is running.
    adb shell input tap "$mx" "$my" >/dev/null 2>&1 || true
    sleep 1
    adb shell input tap "$mx" "$my" >/dev/null 2>&1 || true
    adb shell input tap "$mx" "$my" >/dev/null 2>&1 || true
    sleep 2

    # Re-dumped rather than reusing the dump above, which was taken before any
    # of this and describes the screen with no panel on it.
    after="$(adb exec-out uiautomator dump /dev/tty 2>/dev/null | tr -d '\r' | sed 's/></>\n</g')"
    ours_after="$(printf '%s\n' "$after" | grep "package=\"$package\"" || true)"

    behind=0
    if printf '%s\n' "$ours_after" | grep -qF "$tap_label"; then
        behind=1
    fi
    found=0
    total=0
    while IFS= read -r item; do
        total=$((total + 1))
        if printf '%s\n' "$ours_after" | grep -qF "$item"; then
            found=$((found + 1))
        fi
    done <<EOF
$menu_items
EOF

    echo "panel items in the dump : $found of $total"
    echo "the screen behind       : $([ "$behind" -eq 1 ] && echo "STILL PRESENT" || echo "gone")"
    echo

    # **Both halves, and the order of the branches matters.** "The panel is not
    # there" has to be reported before anything is said about blocking, because a
    # drawer that never opened silences the screen behind it in the most
    # convincing possible way — by not existing — and a check that only asked
    # "is `$tap_label` gone" would call that a pass.
    if [ "$found" -eq 0 ]; then
        echo "VERDICT: the drawer never opened, so this run says nothing about blocking."
        echo
        echo "  Not one of $total panel item(s) is in the dump. The taps above are"
        echo "  synthesised and TalkBack consumes the touch stream, so this is the"
        echo "  expected outcome on many devices rather than a fault."
        echo
        echo "  **Do it by hand:** single-tap \"$(sed -n 's/^pub(crate) const MENU_LABEL: &str = "\(.*\)";$/\1/p' "$screen")\""
        echo "  to focus it, double-tap to press, then run this again with --keep"
        echo "  to dump the screen as it stands."
    elif [ "$found" -lt "$total" ]; then
        echo "VERDICT: the panel is partly there — $found of $total. Inconclusive."
        echo
        echo "  A drawer mid-transition dumps like this. Re-run; if it persists,"
        echo "  the panel is being clipped and that is a layout defect rather than"
        echo "  a semantics one."
    elif [ "$behind" -eq 1 ]; then
        echo "VERDICT: the modal does NOT silence the screen behind it."
        echo
        echo "  The whole panel is published and so is \"$tap_label\", which sits"
        echo "  behind the scrim and should have been cleared by the"
        echo "  BlockSemantics that every ModalBarrier carries unconditionally."
        echo
        echo "  Check the in-app result first: if"
        echo "  a_modal_silences_the_screen_behind_it also failed in"
        echo "  ci/mobile/device-suite.sh, the fault is in vieww's SemanticsTree. If it"
        echo "  passed, the blocked tree is being built and lost on the way"
        echo "  through accesskit_android — a bridge bug, and a different file."
    else
        echo "VERDICT: the modal silences the screen behind it, through the bridge."
        echo
        echo "  All $total panel item(s) published, \"$tap_label\" gone. That is the"
        echo "  BlockSemantics decision of 2026-08-14 confirmed on a device for the"
        echo "  first time — it had never been checked anywhere but a host test."
        echo
        echo "  One thing is still left to ears, and it is the thing a dump cannot"
        echo "  reach: TalkBack leaves a strip of live scrim beside the panel."
        echo "  Touch it. Nothing should be spoken and no focus rectangle should"
        echo "  appear. A drawer is the subject here rather than a dialog exactly"
        echo "  because it leaves that strip to touch."
    fi
fi

if [ "$mode" = "report" ] || [ "$mode" = "activate" ] || [ "$mode" = "modal" ]; then
    echo
    echo "Leaving TalkBack ON so you can swipe. Turn it off with:"
    echo "  ci/mobile/a11y-android.sh --off"
fi
