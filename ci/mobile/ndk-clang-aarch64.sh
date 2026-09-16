#!/usr/bin/env bash
#
# The `aarch64-linux-android` linker, for `.cargo/config.toml`.
#
# A shim rather than an argument because cargo's `linker` key takes a program
# and nothing else — there is nowhere to write the architecture. See
# `ci/mobile/ndk-clang.sh`, which is where all of it actually happens.

set -euo pipefail
exec "$(dirname "$0")/ndk-clang.sh" aarch64 "$@"
