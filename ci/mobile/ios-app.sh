#!/usr/bin/env bash
#
# Build the `ios` example into a .app bundle and put it on a simulator or a
# device.
#
#   ci/mobile/ios-app.sh --sim              # booted simulator, no signing needed
#   ci/mobile/ios-app.sh --device           # a plugged-in, trusted iPhone
#   ci/mobile/ios-app.sh --ipa              # an .ipa for a device cloud to resign
#   ci/mobile/ios-app.sh --sim --debug      # slower vello; see the note below
#
# # `--ipa`, and why it needs no Apple account
#
# It builds for the **device** triple — real arm64, the Metal path the simulator
# cannot run vello on at all — bundles it, ad-hoc signs it, and zips it as an
# .ipa. It installs on nothing by itself: an ad-hoc signature carries no
# provisioning profile, so no iPhone will accept it directly.
#
# That is fine for the one thing it is for. A device cloud (BrowserStack App Live,
# Firebase Test Lab, AWS Device Farm) **re-signs** every upload with its own
# profile, because it has to — it cannot install under a stranger's identity. So
# the signature this produces is a placeholder that gets replaced, and the account
# and certificate that `--device` demands are not needed.
#
# This exists because it is the cheapest known route to the project's largest gap:
# no frame has ever reached an iPhone, real or simulated. The macOS CI runner has
# the Apple SDK, so it can produce this artifact; a human downloads it and uploads
# it to a device cloud. **No Mac and no iPhone has to be bought for that.**
#
# Requires macOS with Xcode. Run it from the workspace root.
#
# # Why this exists rather than an Xcode project
#
# iOS calls `main`, so the Rust side is an ordinary binary and nothing about the
# build needs Xcode's opinion about targets. What iOS *does* need is a bundle —
# a directory holding the executable next to an `Info.plist` — and, for a real
# device, a signature. That is three commands, and an .xcodeproj to hold them
# would be a generated file nobody edits and everybody has to keep in sync.
#
# `ci/mobile/simctl-runner.sh` is the sibling of this for *test* binaries: it spawns
# them in the simulator runtime directly, which works precisely because a test
# binary is not an app and needs no bundle.

set -euo pipefail

if ! command -v xcrun >/dev/null 2>&1; then
    echo "ios-app: xcrun not found — this needs macOS with Xcode." >&2
    echo "  On Linux you can still check it compiles:" >&2
    echo "    cargo build --example ios --target aarch64-apple-ios" >&2
    exit 127
fi

target_kind="sim"
profile="release"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --sim) target_kind="sim" ;;
        --device) target_kind="device" ;;
        --ipa) target_kind="ipa" ;;
        # A debug vello is between ten and thirty times slower, so the default
        # is release for the same reason the desktop harness insists on it: a
        # frame rate measured from `rustc -O0` is a report on the compiler.
        --debug) profile="debug" ;;
        --release) profile="release" ;;
        *) echo "ios-app: unknown argument '$1'" >&2; exit 2 ;;
    esac
    shift
done

case "$target_kind" in
    sim) triple="aarch64-apple-ios-sim" ;;
    # Both are real-device builds. `--ipa` differs only in what happens after the
    # bundle exists: signed ad-hoc and zipped, rather than signed for keeps and
    # installed.
    device | ipa) triple="aarch64-apple-ios" ;;
    *)
        echo "ios-app: unknown target '$target_kind'." >&2
        exit 2
        ;;
esac

bundle_id="dev.vieww.demo"
app_name="vieww"

echo "==> building $triple ($profile)"
if [ "$profile" = "release" ]; then
    cargo build -p vieww-platform-winit --example ios --target "$triple" --release
else
    cargo build -p vieww-platform-winit --example ios --target "$triple"
fi

binary="target/$triple/$profile/examples/ios"
if [ ! -f "$binary" ]; then
    echo "ios-app: expected a binary at $binary and there is none." >&2
    exit 1
fi

app="target/ios/$app_name.app"
rm -rf "$app"
mkdir -p "$app"

# The executable's name inside the bundle must match CFBundleExecutable, and
# `ios` would be a confusing thing to see in a crash report.
cp "$binary" "$app/$app_name"

# UILaunchScreen is not decoration. Without it iOS runs the app in a
# compatibility mode sized for an older device — the window comes out smaller
# than the screen with black bars, which looks exactly like a layout bug in the
# framework and is not one.
cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$app_name</string>
    <key>CFBundleDisplayName</key><string>$app_name</string>
    <key>CFBundleIdentifier</key><string>$bundle_id</string>
    <key>CFBundleExecutable</key><string>$app_name</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>CFBundleShortVersionString</key><string>0.1</string>
    <key>LSRequiresIPhoneOS</key><true/>
    <key>MinimumOSVersion</key><string>13.0</string>
    <key>UILaunchScreen</key><dict/>
    <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string></array>
    <key>UISupportedInterfaceOrientations</key>
    <array><string>UIInterfaceOrientationPortrait</string></array>
</dict>
</plist>
PLIST

echo "==> bundled $app"

if [ "$target_kind" = "ipa" ]; then
    # Ad-hoc (`-`) rather than a real identity: available on any Mac with Xcode
    # and no account at all. The device cloud replaces it.
    #
    # Signed rather than left bare because some resigning pipelines refuse a
    # wholly unsigned Mach-O, and an ad-hoc signature costs nothing.
    codesign --force --sign - --timestamp=none "$app"
    echo "==> ad-hoc signed (a device cloud will re-sign this)"

    # An .ipa is a zip with the bundle under `Payload/`. That layout is the whole
    # format — there is no manifest to get wrong.
    payload="target/ios/Payload"
    ipa="target/ios/$app_name.ipa"
    rm -rf "$payload" "$ipa"
    mkdir -p "$payload"
    cp -R "$app" "$payload/"
    # `-y` preserves the symlinks a framework bundle would have. There are none
    # here today, and there will be the moment anything links dynamically.
    (cd target/ios && zip -qry "$app_name.ipa" Payload)
    rm -rf "$payload"

    echo "==> $ipa"
    echo
    echo "  Upload it to a device cloud, which will re-sign it. It will NOT"
    echo "  install on an iPhone as-is — an ad-hoc signature has no provisioning"
    echo "  profile. Use --device for that, which needs an Apple identity."
    exit 0
fi

if [ "$target_kind" = "sim" ]; then
    device="${VIEWW_SIM_DEVICE:-booted}"
    if [ "$device" = "booted" ] && ! xcrun simctl list devices booted | grep -q Booted; then
        echo "ios-app: no simulator is booted." >&2
        echo "  Boot one first:  xcrun simctl boot 'iPhone 16' && open -a Simulator" >&2
        exit 3
    fi
    # A simulator runs unsigned bundles, which is the whole reason to start here
    # rather than on a device: it separates "does it render" from "is the
    # signing set up", and those fail in ways that look nothing alike.
    xcrun simctl install "$device" "$app"
    echo "==> installed, launching"
    exec xcrun simctl launch --console "$device" "$bundle_id"
fi

# ---------------------------------------------------------------- real device

if [ -z "${VIEWW_IOS_IDENTITY:-}" ]; then
    echo "ios-app: VIEWW_IOS_IDENTITY is unset, and a device will not run an" >&2
    echo "  unsigned bundle. Find yours with:" >&2
    echo "    security find-identity -v -p codesigning" >&2
    echo "  then:" >&2
    echo "    export VIEWW_IOS_IDENTITY='Apple Development: you@example.com (XXXXXXXXXX)'" >&2
    echo "  A free Apple ID is enough; Xcode creates the identity the first time" >&2
    echo "  it builds any app to a device." >&2
    exit 4
fi

# A provisioning profile is what says *this* identity may install *this* bundle
# id on *that* device. Xcode writes them to ~/Library/MobileDevice; the easiest
# way to get one is to let Xcode build a throwaway app to the phone once.
if [ -n "${VIEWW_IOS_PROFILE:-}" ]; then
    cp "$VIEWW_IOS_PROFILE" "$app/embedded.mobileprovision"
else
    echo "ios-app: VIEWW_IOS_PROFILE is unset. Trying without one — this works" >&2
    echo "  only if the identity is tied to a wildcard profile already." >&2
fi

# get-task-allow is what lets a debugger attach and, more importantly here, what
# a development profile expects to see. A mismatch between the entitlements and
# the profile is the usual cause of an install that fails with no useful reason.
entitlements="target/ios/entitlements.plist"
cat > "$entitlements" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>get-task-allow</key><true/>
</dict>
</plist>
PLIST

echo "==> signing as $VIEWW_IOS_IDENTITY"
codesign --force --sign "$VIEWW_IOS_IDENTITY" --entitlements "$entitlements" --timestamp=none "$app"

echo "==> installing"
# `devicectl` is the modern path and needs no third-party tooling. `ios-deploy`
# is the fallback on older Xcode.
if xcrun devicectl list devices >/dev/null 2>&1; then
    udid="${VIEWW_IOS_DEVICE:-$(xcrun devicectl list devices 2>/dev/null \
        | awk 'NR>2 && $1 != "" {print $3; exit}')}"
    if [ -z "$udid" ]; then
        echo "ios-app: no device found. Plug the iPhone in, unlock it, and trust" >&2
        echo "  this Mac. Then:  xcrun devicectl list devices" >&2
        exit 5
    fi
    xcrun devicectl device install app --device "$udid" "$app"
    echo "==> installed. Launch it from the home screen."
    echo "    Logs:  xcrun devicectl device console --device $udid"
elif command -v ios-deploy >/dev/null 2>&1; then
    exec ios-deploy --bundle "$app" --debug
else
    echo "ios-app: neither devicectl nor ios-deploy is available." >&2
    echo "  The signed bundle is at $app — drag it onto the device in Xcode's" >&2
    echo "  Devices window, or install ios-deploy." >&2
    exit 6
fi
