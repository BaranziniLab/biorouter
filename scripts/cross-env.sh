#!/usr/bin/env bash
# scripts/cross-env.sh — THE single source of truth for cross-compilation.
#
# Sourced by scripts/release.sh, the Justfile, and .github/workflows/rust.yml.
# Do NOT inline a docker cross-compile recipe anywhere else — the toolchain
# image pins, the mingw winpthread linker wrap, and LZMA_API_STATIC=1 all live
# here and nowhere else. scripts/check-no-cross-drift.sh fails the build if a
# second copy of the recipe appears.
#
# BR-70: a `cargo check` cross gate (`just check-cross` + the Rust CI workflow)
# and the release's cross *build* must exercise the SAME docker image, the SAME
# toolchain env, and the SAME linker hacks. Extraction — not addition — is what
# keeps the PR gate honest: it can never test something the release does not
# build. (Before this file the Justfile linux recipe had already drifted onto
# `rust:latest`, silently raising the glibc floor the release deliberately pins.)
#
# Functions:
#   cross_linux   <cargo_cmd> [target-dir] [post_cmd]
#   cross_windows <cargo_cmd> [target-dir] [post_cmd]
# Each runs the given cargo command inside the pinned cross image with the full
# toolchain env exported, appends `--target <triple>` to the command, and then
# runs the optional trailing shell snippet (used by the release to stage the
# mingw runtime DLLs). Behaviour-identical to the recipe scripts/release.sh
# shipped before this extraction.
#
# Overridable knobs (env):
#   LINUX_RUST_IMG / WIN_RUST_IMG   cross base images (defaults below)
#   GLIBC_FLOOR                     oldest glibc the linux binaries must run on
#   CROSS_REGISTRY_MOUNT            host path bound to the cargo registry cache
#                                   instead of the docker named volume (CI:
#                                   actions/cache persists it across runs)
#   CROSS_TARGET_MOUNT              host path or Docker volume bound to
#                                   /cross-target (pass /cross-target as the
#                                   target-dir arg to use it)

# ── The pins. THE GLIBC FLOOR LIVES HERE AND NOWHERE ELSE. ────────────────────
# Pin the Linux cross-compile to an OLD-glibc base (Debian 11 "bullseye",
# glibc 2.31) so the produced binaries start on mainstream distros — Ubuntu
# 20.04+/22.04/24.04, Debian 11/12, and RHEL/Rocky 8/9 (glibc 2.34). The rolling
# `rust:latest` (now trixie, glibc 2.39) yields binaries that fail to start on
# anything older than bleeding-edge. Windows (mingw) has no glibc concern.
: "${LINUX_RUST_IMG:=rust:1.92-bullseye}"
: "${WIN_RUST_IMG:=rust:latest}"
: "${GLIBC_FLOOR:=2.31}"

ROOT="${ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# aws-lc-sys (rustls / AWS SDK) compiles a POSIX threading shim under mingw and
# references winpthread symbols (pthread_*); rustc places `-C link-arg=-l…`
# BEFORE the rlibs, so GNU ld discards the lib before it is needed. Wrap the
# linker to append `-lpthread -lwinpthread` AFTER everything else.
WIN_LINKER_WRAP='printf "#!/bin/sh\nexec x86_64-w64-mingw32-gcc \"\$@\" -lpthread -lwinpthread\n" > /usr/local/bin/winpthread-gcc && chmod +x /usr/local/bin/winpthread-gcc'

# lzma-sys (via xz2, the knowledge .brkb path) would otherwise find the HOST
# liblzma through pkg-config and emit an invalid dynamic link. LZMA_API_STATIC=1
# forces it to statically compile its bundled liblzma C source. This matters for
# `cargo check` too — check RUNS build scripts.

# Stage the mingw runtime DLLs next to the built Windows binaries. Used as the
# `post_cmd` of a release `cross_windows` build (not needed for a `cargo check`).
# The `$(...)` / `$GCC_DIR` here are evaluated by the container shell, not the
# host, because this value is substituted verbatim into the `sh -c` string.
WIN_DLL_STAGE='GCC_DIR=$(ls -d /usr/lib/gcc/x86_64-w64-mingw32/*/ | head -n 1) && cp $GCC_DIR/libstdc++-6.dll $GCC_DIR/libgcc_s_seh-1.dll /usr/x86_64-w64-mingw32/lib/libwinpthread-1.dll /usr/src/myapp/target/x86_64-pc-windows-gnu/release/'

# ── internals ────────────────────────────────────────────────────────────────
_cross_need_docker() {
  docker info >/dev/null 2>&1 && return 0
  echo "cross-env: docker daemon not reachable — start Docker and retry" >&2
  return 1
}

# Echo the `-v …:/usr/local/cargo/registry` docker arg. Uses CROSS_REGISTRY_MOUNT
# (a host bind path, for CI caches) when set, else the named volume passed in $1.
_cross_registry_arg() {
  if [ -n "${CROSS_REGISTRY_MOUNT:-}" ]; then
    mkdir -p "$CROSS_REGISTRY_MOUNT"
    printf -- '-v %s:/usr/local/cargo/registry' "$CROSS_REGISTRY_MOUNT"
  else
    docker volume create "$1" >/dev/null 2>&1 || true
    printf -- '-v %s:/usr/local/cargo/registry' "$1"
  fi
}

# ── linux (x86_64-unknown-linux-gnu, glibc-floor pinned) ─────────────────────
cross_linux() {
  local cargo_cmd="$1" tdir="${2:-}" post_cmd="${3:-}"
  _cross_need_docker || return 1
  # shellcheck disable=SC2046  # reg_arg is deliberately word-split into two args
  docker run --rm \
    ${CARGO_BUILD_JOBS:+-e CARGO_BUILD_JOBS} \
    -v "$ROOT":/usr/src/myapp \
    $(_cross_registry_arg biorouter-linux-bullseye-cache) \
    ${CROSS_TARGET_MOUNT:+-v "$CROSS_TARGET_MOUNT":/cross-target} \
    -w /usr/src/myapp "$LINUX_RUST_IMG" sh -c "
      set -e
      rustup target add x86_64-unknown-linux-gnu
      # Debian 11 has passed its LTS end, so deb.debian.org no longer serves the
      # bullseye -security pool: libssl1.1 and linux-libc-dev 404 and the whole
      # install aborts. The image ships the fix already, commented out — its own
      # snapshot.debian.org lines, pinned to a date. Snapshot keeps every version
      # forever, so this is reproducible as well as merely working.
      #
      # DO NOT 'fix' this by bumping LINUX_RUST_IMG. Bullseye is what holds the
      # GLIBC_2.31 floor that Debian 11 / Ubuntu 22.04 / Rocky 9 users depend on;
      # a newer base makes apt succeed and silently raises the floor, and no
      # smoke test in this repo runs on glibc old enough to notice.
      # assert_glibc_floor is the only thing that would catch it.
      sed -i 's|^deb http://deb.debian.org|# deb http://deb.debian.org|; s|^# deb http://snapshot.debian.org|deb http://snapshot.debian.org|' /etc/apt/sources.list
      printf 'Acquire::Check-Valid-Until \"false\";\n' > /etc/apt/apt.conf.d/99no-check-valid-until
      dpkg --add-architecture amd64 && apt-get update -q
      apt-get install -y --no-install-recommends gcc-x86-64-linux-gnu g++-x86-64-linux-gnu \
        protobuf-compiler cmake libxcb1-dev:amd64 libbz2-dev:amd64
      export CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
             CXX_x86_64_unknown_linux_gnu=x86_64-linux-gnu-g++ \
             AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar \
             CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
             LZMA_API_STATIC=1 PKG_CONFIG_ALLOW_CROSS=1 \
             PKG_CONFIG_PATH_x86_64_unknown_linux_gnu=/usr/lib/x86_64-linux-gnu/pkgconfig \
             PROTOC=/usr/bin/protoc
      ${tdir:+export CARGO_TARGET_DIR=$tdir;}
      $cargo_cmd --target x86_64-unknown-linux-gnu
      $post_cmd"
}

# ── windows (x86_64-pc-windows-gnu, mingw) ───────────────────────────────────
cross_windows() {
  local cargo_cmd="$1" tdir="${2:-}" post_cmd="${3:-}"
  _cross_need_docker || return 1
  # shellcheck disable=SC2046
  docker run --rm \
    ${CARGO_BUILD_JOBS:+-e CARGO_BUILD_JOBS} \
    -v "$ROOT":/usr/src/myapp \
    $(_cross_registry_arg biorouter-windows-cache) \
    ${CROSS_TARGET_MOUNT:+-v "$CROSS_TARGET_MOUNT":/cross-target} \
    -w /usr/src/myapp "$WIN_RUST_IMG" sh -c "
      set -e
      rustup target add x86_64-pc-windows-gnu
      apt-get update && apt-get install -y mingw-w64 protobuf-compiler cmake
      $WIN_LINKER_WRAP
      export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
             CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ \
             AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
             CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=/usr/local/bin/winpthread-gcc \
             LZMA_API_STATIC=1 PKG_CONFIG_ALLOW_CROSS=1 PROTOC=/usr/bin/protoc PATH=/usr/bin:\$PATH
      ${tdir:+export CARGO_TARGET_DIR=$tdir;}
      $cargo_cmd --target x86_64-pc-windows-gnu
      $post_cmd"
}
