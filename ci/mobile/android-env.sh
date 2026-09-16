# Resolve the Android SDK and NDK, once, for every script that needs them.
#
# Source this; do not run it:
#
#     . "$(dirname "$0")/android-env.sh"
#     android_env "the-script-name"
#
# It exports ANDROID_HOME, ANDROID_NDK_ROOT and ANDROID_NDK_HOME, and returns
# non-zero with an explanation on stderr if it cannot.
#
# # Why this is shared rather than copied
#
# It was copied, and the copy cost a session. `ci/mobile/device-suite.sh` already knew
# that **a version directory without `source.properties` is a partial download**
# — the SDK manager leaves one behind when an install is abandoned, and
# `ndk/30.0.15729638` on this machine is exactly that. `ci/mobile/reload-guest-android.sh`
# was written later without that knowledge, picked the newest directory by
# version sort, and failed with `Error detecting NDK version for path` — the same
# failure, rediscovered and re-fixed from nothing. Two scripts that answer "which
# NDK" differently will eventually build with different toolchains and blame the
# code.

# Preferred over ANDROID_NDK_HOME when both are set, because `cargo apk` reads
# this one.
android_env() {
    _who="${1:-android-env}"

    # `cargo apk` reads the SDK from the environment and its error for a missing
    # one names the variable but not the usual location. Fill in the default
    # rather than failing, and only complain when the guess is wrong too.
    : "${ANDROID_HOME:=$HOME/Android/Sdk}"
    export ANDROID_HOME
    if [ ! -d "$ANDROID_HOME" ]; then
        echo "$_who: no Android SDK at $ANDROID_HOME." >&2
        echo "  Set ANDROID_HOME to where yours is." >&2
        return 6
    fi

    # In order of preference: what the caller named, then what is installed.
    : "${ANDROID_NDK_ROOT:=${ANDROID_NDK_HOME:-}}"

    if [ -n "$ANDROID_NDK_ROOT" ]; then
        # An explicitly named NDK is validated too, and early: the panic this
        # replaces happens deep inside ndk-build and names neither the directory
        # nor the file it wanted.
        if [ ! -f "$ANDROID_NDK_ROOT/source.properties" ]; then
            echo "$_who: ANDROID_NDK_ROOT=$ANDROID_NDK_ROOT has no source.properties." >&2
            echo "  That is a partial download, not an NDK. Unset it to let this" >&2
            echo "  script choose, or point it at a complete one." >&2
            return 6
        fi
    else
        # `-type d` alone is not enough to call something an NDK. `source.properties`
        # is the right marker: `ndk-build` reads it to learn the revision, so an
        # NDK without one cannot be used by the very tools these scripts run.
        _installed="$(find "$ANDROID_HOME/ndk" -maxdepth 1 -mindepth 1 -type d \
            -exec test -f '{}/source.properties' \; -print 2>/dev/null | sort -V || true)"
        ANDROID_NDK_ROOT="$(echo "$_installed" | tail -1)"
        _count="$(echo "$_installed" | grep -c . || true)"

        if [ -z "$ANDROID_NDK_ROOT" ]; then
            echo "$_who: no usable NDK found under $ANDROID_HOME/ndk." >&2
            echo "  A version directory without source.properties is a partial" >&2
            echo "  download and is skipped. Install one, or set ANDROID_NDK_ROOT." >&2
            return 6
        fi

        # Said out loud, because silently swapping the toolchain is how a build
        # that worked yesterday fails today for a reason nobody can see.
        if [ "${_count:-0}" -gt 1 ]; then
            echo "$_who: $_count usable NDKs installed; using the newest." >&2
            echo "$_installed" | sed 's|.*/|    |' >&2
            echo "  Set ANDROID_NDK_ROOT to pin a different one." >&2
        fi
    fi

    export ANDROID_NDK_ROOT
    # cargo-ndk reads this one rather than ANDROID_NDK_ROOT. Kept in step so the
    # two tools cannot disagree about which NDK is in use.
    export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"

    # `cargo apk` signs debug builds with the NDK's own key and **refuses to
    # guess for release**. These env vars beat the manifest, which is the only
    # way to use a keystore outside the crate directory: `signing.release.path`
    # is joined to the crate dir, so following the error message literally means
    # committing a keystore.
    #
    # Every crate that ships an APK needs this, which is the third reason this
    # file exists: `ci/mobile/reload-android.sh` was written without it and failed on
    # `Configure a release keystore via [package.metadata.android.signing.release]`
    # — an error that points at the manifest, where the fix cannot go.
    if [ -z "${CARGO_APK_RELEASE_KEYSTORE:-}" ] && [ -f "$HOME/.android/debug.keystore" ]; then
        export CARGO_APK_RELEASE_KEYSTORE="$HOME/.android/debug.keystore"
        export CARGO_APK_RELEASE_KEYSTORE_PASSWORD="${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:-android}"
    fi

    _revision="$(sed -n 's/^Pkg\.Revision *= *//p' "$ANDROID_NDK_ROOT/source.properties" | head -1)"
    echo "$_who: SDK $ANDROID_HOME, NDK ${_revision:-unknown} at $ANDROID_NDK_ROOT"
}
