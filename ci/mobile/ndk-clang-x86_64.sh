#!/usr/bin/env bash
#
# The `x86_64-linux-android` linker, for `.cargo/config.toml`.
#
# This is the emulator's architecture, not a phone's — see `ci/mobile/ndk-clang.sh`
# for everything this defers to, and `ci/mobile/adb-runner.sh` for the guard that says
# so when a binary meets the wrong device.

set -euo pipefail
exec "$(dirname "$0")/ndk-clang.sh" x86_64 "$@"
