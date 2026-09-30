#!/usr/bin/env bash
# Native package installs share the canonical desktop assertions.
set -euo pipefail
FORMAT="${1:?usage: smoke-native-linux-packages.sh deb|rpm <package-dir> <smoke-script>}"
PACKAGES="${2:?package directory required}"
SMOKE="${3:?desktop smoke script required}"
[ "$(uname -m)" = x86_64 ]
[ "$(id -u)" = 0 ]
(cd "$PACKAGES" && sha256sum --check packages.sha256)
export BIOROUTER_DISABLE_KEYRING=true
unset BIOROUTER_DEV_PROFILE_ROOT BIOROUTER_DEV_PROFILE_NAME ENABLE_PLAYWRIGHT BIOROUTER_SHARED_DAEMON BIOROUTER_PATH_ROOT
test "$(stat -c %u:%g:%a /var/lib)" = 0:0:755
QA_USER=biorouter_package_qa
QA_HOME=/var/lib/biorouter-native-package-home
! id "$QA_USER" >/dev/null 2>&1
[ ! -e "$QA_HOME" ]
useradd --uid 64011 --create-home --home-dir "$QA_HOME" --shell /bin/bash "$QA_USER"
chmod 700 "$QA_HOME"
install -m 0755 "$SMOKE" /var/lib/biorouter-native-package-smoke.sh
SMOKE=/var/lib/biorouter-native-package-smoke.sh
qa() { runuser -u "$QA_USER" -- env -i HOME="$QA_HOME" PATH=/usr/sbin:/usr/bin:/sbin:/bin BIOROUTER_DISABLE_KEYRING=true "$@"; }
namespace_error=$(mktemp /tmp/biorouter-userns-XXXXXX)
if qa unshare --user --map-root-user true 2>"$namespace_error"; then
  sandbox_mode=enabled
  sandbox_switch=0
else
  grep -Eq 'Operation not permitted|Permission denied' "$namespace_error"
  sandbox_mode=disabled-fixture-user-namespace-unavailable
  sandbox_switch=1
fi
rm "$namespace_error"
printf 'CHROMIUM_SANDBOX_MODE=%s\n' "$sandbox_mode"
case "$FORMAT" in
  deb)
    apt-get update -qq
    apt-get install -y -qq "$PACKAGES/biorouter_1.92.0_amd64.deb" xvfb
    resources=/usr/lib/biorouter/resources/bin
    qa "$resources/biorouter" --version | grep -Eq '(^| )1\.92\.0$'
    qa "$resources/biorouterd" --version | grep -Eq '(^| )1\.92\.0$'
    qa timeout -k 10 600 env BR_SMOKE_DISABLE_CHROMIUM_SANDBOX="$sandbox_switch" bash "$SMOKE" /usr/bin/biorouter
    dpkg --remove biorouter
    apt-get install -y -qq "$PACKAGES/biorouter-cli_1.92.0_amd64.deb"
    ;;
  rpm)
    dnf install -y -q "$PACKAGES/Biorouter-1.92.0-1.x86_64.rpm" xorg-x11-server-Xvfb
    resources=/usr/lib/Biorouter/resources/bin
    qa "$resources/biorouter" --version | grep -Eq '(^| )1\.92\.0$'
    qa "$resources/biorouterd" --version | grep -Eq '(^| )1\.92\.0$'
    qa timeout -k 10 600 env BR_SMOKE_DISABLE_CHROMIUM_SANDBOX="$sandbox_switch" bash "$SMOKE" /usr/bin/Biorouter
    rpm --erase Biorouter
    dnf install -y -q "$PACKAGES/biorouter-cli-1.92.0-1.x86_64.rpm"
    ;;
  *) exit 2 ;;
esac
qa biorouter --version | grep -Eq '(^| )1\.92\.0$'
qa biorouterd --version | grep -Eq '(^| )1\.92\.0$'
qa biorouter-crew --version | grep -Fx 'biorouter-crew 1.92.0'
qa biorouter term --help >/dev/null
printf 'NATIVE_LINUX_PACKAGE_ACCEPTANCE_OK %s\n' "$FORMAT"
