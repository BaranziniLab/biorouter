#!/bin/sh
set -eu

json_str() {
  printf '%s' "$1" | LC_ALL=C awk '
    BEGIN { RS = "\001" }
    {
      out = ""
      n = length($0)
      for (i = 1; i <= n; i++) {
        c = substr($0, i, 1)
        if (c == "\\" || c == "\"") out = out "\\" c
        else if (c == "\t") out = out "\\t"
        else if (c == "\r") out = out "\\r"
        else if (c == "\n") out = out "\\n"
        else if (c !~ /[[:cntrl:]]/) out = out c
      }
      printf "%s", out
    }
  '
}

emit_ok() {
  printf '{"ok":true,"state":"%s","message":"%s"' "$1" "$(json_str "$2")"
  if [ -n "${3:-}" ]; then
    printf ',"next":"%s"' "$(json_str "$3")"
  fi
  printf '}\n'
}

emit_fail() {
  printf '{"ok":false,"code":"%s","message":"%s","suggestion":"%s"}\n' \
    "$1" "$(json_str "$2")" "$(json_str "$3")"
}

AD_SANDBOX_DIR=${AD_SANDBOX_DIR:-}
AD_PROVIDER_DIR=${AD_PROVIDER_DIR:-}
AD_BINARY=${AD_BINARY:-}
# The built VM host is keyed by its own source, so an agent-desktop upgrade that leaves it unchanged reuses it.
vmhost_key=$(cat "$AD_PROVIDER_DIR/vmhost.swift" "$AD_PROVIDER_DIR/vm.entitlements" 2>/dev/null | shasum -a 1 | cut -c1-16)
VMHOST=${VMHOST:-$(dirname "$AD_PROVIDER_DIR")/build-$vmhost_key/vmhost}
HELPER_NAME=agent-desktop-macos-helper
SCRIPT_PATH=$(cd "$(dirname "$0")" && pwd)/$(basename "$0")
verb=${1:-}
if [ "$#" -gt 0 ]; then shift; fi

require_provider_environment() {
  if [ -z "$AD_PROVIDER_DIR" ]; then
    emit_fail INVALID_ARGS "agent-desktop did not pass the provider directory this script needs." "Run an agent-desktop sandbox command instead of calling the provider directly."
    exit 1
  fi
  BUILD_DIR=$(dirname "$VMHOST")
  BUILD_LOG=$BUILD_DIR/build.log
}

require_sandbox_environment() {
  require_provider_environment
  if [ -z "$AD_SANDBOX_DIR" ]; then
    emit_fail INVALID_ARGS "agent-desktop did not pass the sandbox directory this script needs." "Run an agent-desktop sandbox command instead of calling the provider directly."
    exit 1
  fi
  VM_DIR=$AD_SANDBOX_DIR/vm
}

host_helper_path() {
  printf '%s\n' "$(dirname "$AD_BINARY")/$HELPER_NAME"
}

require_host_package() {
  suggestion=$1
  if [ ! -f "$AD_BINARY" ]; then
    emit_fail ACTION_FAILED "The host agent-desktop binary is missing." "$suggestion"
    return 1
  fi
  host_helper=$(host_helper_path)
  if [ ! -f "$host_helper" ] || [ ! -x "$host_helper" ]; then
    emit_fail ACTION_FAILED "The host agent-desktop package is incomplete: $HELPER_NAME is missing or not executable beside agent-desktop." "$suggestion"
    return 1
  fi
}

valid_name() {
  case $1 in
    ''|.|..|*/*) return 1 ;;
    *) return 0 ;;
  esac
}

valid_guest_user() {
  case $1 in
    ''|-*|*[!A-Za-z0-9._-]*) return 1 ;;
    *) return 0 ;;
  esac
}

resolve_image_path() {
  case $1 in
    /*) printf '%s\n' "$1" ;;
    '') return 1 ;;
    *)
      if [ -z "${AD_CALLER_CWD:-}" ]; then return 2; fi
      printf '%s\n' "$AD_CALLER_CWD/$1"
      ;;
  esac
}

host_runtime_supported() {
  architecture=$(uname -m 2>/dev/null || true)
  if [ "$architecture" != arm64 ]; then
    emit_ok unavailable "macOS VMs need an Apple Silicon Mac." "Use an Apple Silicon Mac, then run agent-desktop sandbox install macos."
    return 1
  fi
  if [ "$(sysctl -n kern.hv_support 2>/dev/null || true)" != 1 ]; then
    emit_ok unavailable "This Mac cannot run virtual machines (it may itself be a virtual machine)."
    return 1
  fi
  version=$(sw_vers -productVersion 2>/dev/null || true)
  major=${version%%.*}
  case $major in ''|*[!0-9]*) major=0 ;; esac
  if [ "$major" -lt 13 ]; then
    emit_ok unavailable "macOS VMs need macOS 13 or later on the host." "Upgrade this Mac to macOS 13 or later, then run agent-desktop sandbox install macos."
    return 1
  fi
  return 0
}

build_tools_available() {
  if ! xcrun --find swiftc >/dev/null 2>&1; then
    emit_ok unavailable "Xcode Command Line Tools are not installed." "xcode-select --install"
    return 1
  fi
  return 0
}

provider_ready() {
  [ -f "$BUILD_DIR/vmhost" ] || return 1
  if ! entitlements=$(codesign -d --entitlements - "$BUILD_DIR/vmhost" 2>&1); then return 1; fi
  case $entitlements in
    *com.apple.security.virtualization*) return 0 ;;
    *) return 1 ;;
  esac
}

do_check() {
  if ! host_runtime_supported; then return 0; fi
  if provider_ready; then
    emit_ok ready "The macOS VM provider is ready."
    return 0
  fi
  if ! build_tools_available; then return 0; fi
  emit_ok missing "The macOS VM host has not been built and signed." "agent-desktop sandbox install macos"
}

do_install() {
  if provider_ready; then
    emit_ok ready "The macOS VM provider is ready."
    return 0
  fi
  if ! host_runtime_supported; then return 0; fi
  if ! build_tools_available; then return 0; fi
  if ! mkdir -p "$BUILD_DIR" || ! : > "$BUILD_LOG"; then
    emit_fail ACTION_FAILED "The macOS VM build directory is not writable." "Make $AD_PROVIDER_DIR writable, then run agent-desktop sandbox install macos."
    return 1
  fi
  if [ ! -f "$AD_PROVIDER_DIR/vmhost.swift" ] || [ ! -f "$AD_PROVIDER_DIR/vm.entitlements" ]; then
    printf '%s\n' "The VM host source or entitlements file is missing from $AD_PROVIDER_DIR." >> "$BUILD_LOG"
    emit_fail ACTION_FAILED "The macOS VM host could not be installed; see $BUILD_LOG." "Read $BUILD_LOG, then run agent-desktop sandbox install macos again."
    return 1
  fi
  if ! xcrun swiftc -O "$AD_PROVIDER_DIR/vmhost.swift" -o "$BUILD_DIR/vmhost" >> "$BUILD_LOG" 2>&1; then
    rm -f "$BUILD_DIR/vmhost"
    emit_fail ACTION_FAILED "The macOS VM host could not be compiled; see $BUILD_LOG." "Read $BUILD_LOG, then run agent-desktop sandbox install macos again."
    return 1
  fi
  if ! codesign -f -s - --entitlements "$AD_PROVIDER_DIR/vm.entitlements" "$BUILD_DIR/vmhost" >> "$BUILD_LOG" 2>&1; then
    rm -f "$BUILD_DIR/vmhost"
    emit_fail ACTION_FAILED "The macOS VM host could not be signed; see $BUILD_LOG." "Read $BUILD_LOG, then run agent-desktop sandbox install macos again."
    return 1
  fi
  if ! provider_ready; then
    rm -f "$BUILD_DIR/vmhost"
    printf '%s\n' 'The signed VM host does not contain the virtualization entitlement.' >> "$BUILD_LOG"
    emit_fail ACTION_FAILED "The macOS VM host is missing its virtualization entitlement; see $BUILD_LOG." "Read $BUILD_LOG, then run agent-desktop sandbox install macos again."
    return 1
  fi
  emit_ok ready "The macOS VM host was built and signed; the provider is ready."
}

vm_bundle_complete() {
  [ -f "$1/Disk.img" ] && [ -f "$1/AuxiliaryStorage" ] && \
    [ -f "$1/HardwareModel" ] && [ -f "$1/MachineIdentifier" ] && [ -f "$1/config.json" ] && \
    [ -f "$1/share/guest-setup.sh" ] && [ -f "$1/share/id_ed25519.pub" ] && \
    [ -f "$1/share/agent-desktop" ]
}

target_has_sandbox_data() {
  [ -e "$VM_DIR" ] || [ -e "$AD_SANDBOX_DIR/id_ed25519" ] || \
    [ -e "$AD_SANDBOX_DIR/id_ed25519.pub" ] || [ -e "$AD_SANDBOX_DIR/known_hosts" ] || \
    [ -e "$AD_SANDBOX_DIR/guest_user" ] || [ -e "$AD_SANDBOX_DIR/setup_done" ]
}

vmhost_is_live() {
  live_pid=$(cat "$1/vmhost.pid" 2>/dev/null || true)
  case $live_pid in ''|*[!0-9]*) return 1 ;; esac
  live_command=$(ps -ww -o command= -p "$live_pid" 2>/dev/null | tr -d '\r' | sed -e 's/[[:space:]]*$//' || true)
  case $live_command in
    *"vmhost run $1"|*"vmhost run $1 --window"*) return 0 ;;
    *) return 1 ;;
  esac
}

job_is_running() {
  job_pid=$(cat "$VM_DIR/install.pid" 2>/dev/null || true)
  case $job_pid in ''|*[!0-9]*) return 1 ;; esac
  pgrep -g "$job_pid" -f -- "$VM_DIR" >/dev/null 2>&1
}

# The one place a sandbox's phase is worked out from its files. Every verb switches on it.
#   installing  the install job is alive
#   failed      the install job ended without finishing, or VM files are missing
#   stopped     the VM files are complete and no VM host runs them
#   starting    the VM host is alive and has not booted the guest yet
#   running     the VM host has booted the guest
sandbox_phase() {
  phase_vm=$1/vm
  if [ ! -f "$phase_vm/install.done" ] && [ -f "$phase_vm/install.pid" ]; then
    if VM_DIR=$phase_vm job_is_running; then printf 'installing\n'; else printf 'failed\n'; fi
  elif ! vm_bundle_complete "$phase_vm"; then printf 'failed\n'
  elif ! vmhost_is_live "$phase_vm"; then printf 'stopped\n'
  elif [ -f "$phase_vm/vmhost.started" ]; then printf 'running\n'
  else printf 'starting\n'
  fi
}

# Answers the phases that every verb reports the same way. Returns 1 for a phase the verb handles itself.
report_phase() {
  case $2 in
    installing)
      progress=$(last_progress)
      if [ -n "$progress" ]; then
        emit_ok working "Sandbox $1 is installing macOS; the latest progress is $progress." "agent-desktop sandbox status $1"
      elif [ -f "$AD_SANDBOX_DIR/restore.ipsw" ]; then
        emit_ok working "Sandbox $1 is downloading the macOS image: $(gigabytes "$(stat -f %z "$AD_SANDBOX_DIR/restore.ipsw" 2>/dev/null || printf '0')") GB so far." "agent-desktop sandbox status $1"
      else
        emit_ok working "Sandbox $1 is preparing to install macOS." "agent-desktop sandbox status $1"
      fi
      ;;
    failed)
      install_reason=
      [ ! -s "$VM_DIR/install.log" ] || install_reason=" The reason is in $VM_DIR/install.log; read it before removing the sandbox."
      emit_ok broken "Sandbox $1 has incomplete or failed VM installation files.$install_reason" "agent-desktop sandbox rm $1"
      ;;
    starting) emit_ok working "Sandbox $1 is starting." "agent-desktop sandbox status $1" ;;
    *) return 1 ;;
  esac
}

last_progress() {
  [ -f "$VM_DIR/install.log" ] || return 0
  grep -Eo '[0-9]+%' "$VM_DIR/install.log" 2>/dev/null | tail -n 1 || true
}

gigabytes() {
  awk -v bytes="$1" 'BEGIN { printf "%.1f", bytes / 1000000000 }'
}

catalog_firmware() {
  tr '<' '\n' < "$1" | awk -v host_major="$2" '
    function newer(candidate, current, candidate_parts, current_parts, candidate_count, current_count, i) {
      if (current == "") return 1
      candidate_count = split(candidate, candidate_parts, /[.]/)
      current_count = split(current, current_parts, /[.]/)
      for (i = 1; i <= candidate_count || i <= current_count; i++) {
        if (candidate_parts[i] + 0 > current_parts[i] + 0) return 1
        if (candidate_parts[i] + 0 < current_parts[i] + 0) return 0
      }
      return 0
    }
    /^key>Restore$/ { awaiting_restore = 1; next }
    awaiting_restore && /^dict>$/ { in_restore = 1; awaiting_restore = 0; next }
    in_restore && /^key>(FirmwareURL|FirmwareSHA1|ProductVersion)$/ {
      field = substr($0, 5)
      next
    }
    in_restore && /^string>/ {
      value = substr($0, 8)
      if (field == "FirmwareURL") url = value
      if (field == "FirmwareSHA1") sha = value
      if (field == "ProductVersion") version = value
      field = ""
      next
    }
    in_restore && /^\/dict>/ {
      if (url != "" && sha != "" && version ~ /^[0-9]+([.][0-9]+)*$/ && int(version) <= host_major && newer(version, best_version)) {
        best_url = url
        gsub(/&amp;/, "\\&", best_url)
        best_sha = sha
        best_version = version
      }
      in_restore = 0
      url = ""
      sha = ""
      version = ""
      field = ""
    }
    END {
      if (best_url != "" && best_sha != "") {
        print best_url
        print best_sha
      }
    }
  '
}

copy_guest_share() {
  share=$1/share
  host_helper=$(host_helper_path)
  if [ ! -f "$AD_BINARY" ] || [ ! -f "$host_helper" ] || [ ! -x "$host_helper" ]; then
    return 1
  fi
  mkdir -p "$share" || return 1
  cp "$AD_PROVIDER_DIR/guest-setup.sh" "$share/guest-setup.sh" || return 1
  cp "$AD_SANDBOX_DIR/id_ed25519.pub" "$share/id_ed25519.pub" || return 1
  cp "$AD_BINARY" "$share/agent-desktop" || return 1
  cp "$host_helper" "$share/$HELPER_NAME" || return 1
  chmod 0755 "$share/guest-setup.sh" "$share/agent-desktop" "$share/$HELPER_NAME" || return 1
}

create_guest_identity() {
  guest_user=${AD_SET_USER:-}
  if [ -z "$guest_user" ]; then guest_user=$(id -un 2>/dev/null || true); fi
  if [ -z "$guest_user" ]; then
    emit_fail ACTION_FAILED "The guest user name could not be determined." "Run agent-desktop sandbox create $1 --set user=<name>."
    return 1
  fi
  if ! valid_guest_user "$guest_user"; then
    emit_fail INVALID_ARGS "The guest user name contains unsupported characters." "Run agent-desktop sandbox create $1 --set user=<short-name>."
    return 1
  fi
  if ! ssh-keygen -t ed25519 -N '' -f "$AD_SANDBOX_DIR/id_ed25519" >/dev/null 2>&1; then
    emit_fail ACTION_FAILED "The SSH key for sandbox $1 could not be created." "Run agent-desktop sandbox create $1 again."
    return 1
  fi
  if ! chmod 0600 "$AD_SANDBOX_DIR/id_ed25519" || \
    ! : > "$AD_SANDBOX_DIR/known_hosts" || \
    ! printf '%s\n' "$guest_user" > "$AD_SANDBOX_DIR/guest_user"; then
    emit_fail ACTION_FAILED "The SSH settings for sandbox $1 could not be saved securely." "Run agent-desktop sandbox create $1 again after checking directory permissions."
    return 1
  fi
}

file_mtime() {
  stat -f %m "$1" 2>/dev/null || stat -c %Y "$1" 2>/dev/null || printf '0\n'
}

verify_image_sha() {
  verify_sha=$(shasum -a 1 "$1" 2>/dev/null | awk '{ print tolower($1) }' || true)
  [ -n "$verify_sha" ] && [ "$verify_sha" = "$2" ]
}

latest_image() {
  catalog=$1
  host_version=$(sw_vers -productVersion 2>/dev/null || true)
  if ! curl -fsSL --connect-timeout 30 --speed-limit 1 --speed-time 120 -o "$catalog" 'https://mesu.apple.com/assets/macos/com_apple_macOSIPSW/com_apple_macOSIPSW.xml'; then
    printf '%s\n' 'The Apple macOS image catalog could not be downloaded.'
    return 1
  fi
  firmware=$(catalog_firmware "$catalog" "${host_version%%.*}")
  rm -f "$catalog"
  url=$(printf '%s\n' "$firmware" | sed -n '1p')
  expected=$(printf '%s\n' "$firmware" | sed -n '2p' | tr 'A-F' 'a-f')
  case $url in https://*) ;; *) url= ;; esac
  case $expected in *[!0-9a-f]*) expected= ;; esac
  if [ -z "$url" ] || [ "${#expected}" -ne 40 ]; then
    printf '%s\n' "The Apple macOS image catalog lists no image this Mac running macOS $host_version can install."
    return 1
  fi
  printf '%s\n%s\n' "$url" "$expected"
}

download_image() {
  if ! curl -fsSL --connect-timeout 30 --speed-limit 1 --speed-time 120 -o "$3" "$1"; then
    printf '%s\n' 'The Apple macOS image could not be downloaded.'
    return 1
  fi
  if ! verify_image_sha "$3" "$2"; then
    printf '%s\n' 'The downloaded Apple macOS image did not match its published SHA-1.'
    return 1
  fi
}

do_install_job() {
  ipsw=$2
  if [ "$ipsw" = latest ]; then
    ipsw=$AD_SANDBOX_DIR/restore.ipsw
    if [ -z "${AD_IMAGE_URL:-}" ] || [ -z "${AD_IMAGE_SHA1:-}" ]; then
      if ! resolved=$(latest_image "$AD_SANDBOX_DIR/restore.ipsw.catalog"); then
        rm -f "$ipsw"
        printf '%s\n' "$resolved"
        return 1
      fi
      AD_IMAGE_URL=$(printf '%s\n' "$resolved" | sed -n '1p')
      AD_IMAGE_SHA1=$(printf '%s\n' "$resolved" | sed -n '2p')
      export AD_IMAGE_URL AD_IMAGE_SHA1
    fi
    if ! download_image "$AD_IMAGE_URL" "$AD_IMAGE_SHA1" "$ipsw"; then rm -f "$ipsw"; return 1; fi
  fi
  if ! "$VMHOST" install "$VM_DIR" "$ipsw"; then return 1; fi
  rm -f "$AD_SANDBOX_DIR/restore.ipsw"
  if ! copy_guest_share "$VM_DIR"; then
    printf '%s\n' 'The guest setup files could not be copied into the shared folder.'
    return 1
  fi
  : > "$VM_DIR/install.done"
}

start_install_job() {
  name=$1
  if ! mkdir -p "$VM_DIR" || ! : > "$VM_DIR/install.log"; then
    emit_fail ACTION_FAILED "The installation log for sandbox $name could not be created." "Run agent-desktop sandbox status $name."
    return 1
  fi
  set -m
  nohup sh "$SCRIPT_PATH" __install_job "$VM_DIR" "$2" >> "$VM_DIR/install.log" 2>&1 < /dev/null &
  install_job_pid=$!
  set +m
  if ! printf '%s\n' "$install_job_pid" > "$VM_DIR/install.pid"; then
    kill -TERM "$install_job_pid" 2>/dev/null || true
    emit_fail ACTION_FAILED "The installation process for sandbox $name could not be recorded." "Run agent-desktop sandbox status $name."
    return 1
  fi
  emit_ok working "Sandbox $name is installing macOS." "agent-desktop sandbox status $name"
}

reject_empty_setting() {
  if [ "$1" = x ] && [ -z "$2" ]; then
    emit_fail INVALID_ARGS "The $3 setting has no value." "Run agent-desktop sandbox create $4 --set $3=<value>."
    return 1
  fi
}

# reject_unknown_settings "<allowed keys>": refuses any --set key the verb does not take.
reject_unknown_settings() {
  for setting_name in $(env | sed -n 's/^AD_SET_\([A-Z0-9_]*\)=.*/\1/p' | tr 'A-Z' 'a-z'); do
    case " $1 " in
      *" $setting_name "*) ;;
      *)
        emit_fail INVALID_ARGS "Unknown setting $setting_name." "Pass --set with one of these keys: $1."
        return 1
        ;;
    esac
  done
}

do_create() {
  name=$1
  reject_unknown_settings "ipsw from user" || return 1
  reject_empty_setting "${AD_SET_IPSW+x}" "${AD_SET_IPSW:-}" ipsw "$name" || return 1
  reject_empty_setting "${AD_SET_FROM+x}" "${AD_SET_FROM:-}" from "$name" || return 1
  reject_empty_setting "${AD_SET_USER+x}" "${AD_SET_USER:-}" user "$name" || return 1
  case ${AD_SET_FROM:+from}${AD_SET_IPSW:+ipsw} in
    from|ipsw) ;;
    *)
      emit_fail INVALID_ARGS "The create verb needs exactly one source: an image to install or a sandbox to clone." "Run agent-desktop sandbox create $name --provider macos with --set ipsw=latest, --set ipsw=<path>, or --set from=<sandbox>."
      return 1
      ;;
  esac
  if [ -n "${AD_SET_FROM:-}" ] && [ -n "${AD_SET_USER:-}" ]; then
    emit_fail INVALID_ARGS "The user setting applies only to an install; a clone keeps its source's guest user." "Run agent-desktop sandbox create $name --set from=$AD_SET_FROM without --set user."
    return 1
  fi
  if target_has_sandbox_data; then
    emit_fail ACTION_FAILED "Sandbox $name already has provider data." "Run agent-desktop sandbox status $name."
    return 1
  fi
  if ! host_runtime_supported; then return 0; fi
  if ! provider_ready; then
    vmhost_missing
    return 1
  fi
  if ! require_host_package "Run agent-desktop sandbox create $name again after rebuilding agent-desktop with its helper."; then
    return 1
  fi
  if [ -n "${AD_SET_FROM:-}" ]; then
    create_clone "$name" "$AD_SET_FROM"
    return $?
  fi
  image_path=latest
  if [ "$AD_SET_IPSW" != latest ]; then
    image_status=0
    image_path=$(resolve_image_path "$AD_SET_IPSW") || image_status=$?
    if [ "$image_status" -eq 2 ]; then
      emit_fail INVALID_ARGS "The supplied macOS image path is relative and agent-desktop did not pass the caller directory." "Run agent-desktop sandbox create $name --set ipsw=<absolute-path>."
      return 1
    fi
    if [ ! -f "$image_path" ]; then
      emit_fail INVALID_ARGS "The supplied macOS image file was not found." "Run agent-desktop sandbox create $name --set ipsw=<path> with an existing image file, or --set ipsw=latest to download one."
      return 1
    fi
  fi
  guest_name=${AD_SET_USER:-$(id -un 2>/dev/null || true)}
  if [ -z "$guest_name" ]; then
    emit_fail ACTION_FAILED "The guest user name could not be determined." "Run agent-desktop sandbox create $name --set user=<name>."
    return 1
  fi
  if ! valid_guest_user "$guest_name"; then
    emit_fail INVALID_ARGS "The guest user name contains unsupported characters." "Run agent-desktop sandbox create $name --set user=<short-name>."
    return 1
  fi
  if [ "$image_path" = latest ]; then
    if ! resolved=$(latest_image "$AD_SANDBOX_DIR/restore.ipsw.catalog"); then
      emit_fail ACTION_FAILED "$resolved" "Run agent-desktop sandbox create $name --set ipsw=<path> with a macOS image this Mac can install, or update macOS on this host."
      return 1
    fi
    AD_IMAGE_URL=$(printf '%s\n' "$resolved" | sed -n '1p')
    AD_IMAGE_SHA1=$(printf '%s\n' "$resolved" | sed -n '2p')
    export AD_IMAGE_URL AD_IMAGE_SHA1
  fi
  if ! create_guest_identity "$name"; then return 1; fi
  start_install_job "$name" "$image_path"
}

create_clone() {
  name=$1
  source_name=$2
  source=$(dirname "$AD_SANDBOX_DIR")/$source_name
  if ! valid_name "$source_name" || [ ! -d "$source" ]; then
    emit_fail SANDBOX_NOT_FOUND "No sandbox named $source_name exists." "Run \`agent-desktop sandbox list\` to see the sandboxes that exist."
    return 1
  fi
  source_phase=$(sandbox_phase "$source")
  if [ "$source_phase" != stopped ] || [ ! -f "$source/setup_done" ]; then
    [ "$source_phase" != stopped ] || source_phase='not set up'
    emit_fail ACTION_FAILED "Sandbox $source_name is $source_phase, and only a stopped, set-up sandbox can be cloned." "Run agent-desktop sandbox status $source_name."
    return 1
  fi
  if ! cp "$source/id_ed25519" "$source/id_ed25519.pub" "$source/known_hosts" "$source/guest_user" "$source/setup_done" "$AD_SANDBOX_DIR/" || \
    ! chmod 0600 "$AD_SANDBOX_DIR/id_ed25519"; then
    emit_fail ACTION_FAILED "The SSH settings could not be copied from sandbox $source_name." "Run agent-desktop sandbox status $source_name."
    return 1
  fi
  if ! clone_reply=$("$VMHOST" clone "$source/vm" "$VM_DIR" 2>&1); then
    emit_fail ACTION_FAILED "The VM for sandbox $name could not be cloned: $clone_reply" "Run agent-desktop sandbox rm $name, fix the cause, then create it again."
    return 1
  fi
  rm -f "$AD_SANDBOX_DIR/known_hosts" "$AD_SANDBOX_DIR/vm/host_key.pub"
  rotate_status=0
  rotate_host_key || rotate_status=$?
  if [ "$rotate_status" -ne 0 ]; then
    emit_fail ACTION_FAILED "Sandbox $name could not be given its own SSH host key." "Run agent-desktop sandbox rm $name and create it again; a guest disk that cannot be opened cannot be cloned safely."
    return 1
  fi
  if ! copy_guest_share "$VM_DIR"; then
    emit_fail ACTION_FAILED "The guest setup files could not be copied into sandbox $name." "Run agent-desktop sandbox status $name."
    return 1
  fi
  emit_ok stopped "Sandbox $name is a stopped copy of sandbox $source_name." "agent-desktop sandbox start $name"
}

guest_user() {
  [ -s "$AD_SANDBOX_DIR/guest_user" ] || return 1
  stored_guest_user=$(cat "$AD_SANDBOX_DIR/guest_user" 2>/dev/null || true)
  valid_guest_user "$stored_guest_user" || return 1
  printf '%s\n' "$stored_guest_user"
}

guest_transport() {
  transport_tool=$1
  shift
  host_key_check=accept-new
  [ -s "$AD_SANDBOX_DIR/vm/host_key.pub" ] && host_key_check=yes
  "$transport_tool" -F /dev/null -i "$AD_SANDBOX_DIR/id_ed25519" \
    -o IdentitiesOnly=yes \
    -o "StrictHostKeyChecking=$host_key_check" \
    -o "UserKnownHostsFile=$AD_SANDBOX_DIR/known_hosts" \
    -o BatchMode=yes \
    -o ConnectTimeout=5 \
    -o ConnectionAttempts=1 \
    -o ServerAliveInterval=3 \
    -o ServerAliveCountMax=3 \
    "$@"
}

detach_guest_disk() {
  disk=$1
  for dev in $(hdiutil info 2>/dev/null | awk -v img="$disk" '$1 == "image-path" { path = $0; sub(/^[^:]*:[[:space:]]*/, "", path); found = (path == img) } found && $1 ~ /^\/dev\/disk/ { print $1 }'); do
    hdiutil detach "$dev" >/dev/null 2>&1 || true
  done
}

guest_ssh_dir() {
  attach=$1
  mounts=$2
  printf '%s\n' "$attach" | awk -F '\t' '$3 ~ /^\// { print $3 }' > "$mounts"
  while IFS= read -r mount_point; do
    if [ -f "$mount_point/private/etc/ssh/ssh_host_ed25519_key" ]; then
      printf '%s\n' "$mount_point"
      return 0
    fi
  done < "$mounts"
  return 1
}

rotate_host_key() {
  disk=$AD_SANDBOX_DIR/vm/Disk.img
  pub=$AD_SANDBOX_DIR/vm/host_key.pub
  [ -s "$pub" ] && return 0
  [ -f "$disk" ] || return 2
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/ad-host-key.XXXXXX") || return 1
  if ! ssh-keygen -q -t ed25519 -N '' -C agent-desktop-sandbox-host -f "$tmp/key" >/dev/null 2>&1; then
    rm -rf "$tmp"
    return 1
  fi
  detach_guest_disk "$disk"
  if ! attach=$(hdiutil attach "$disk" -imagekey diskimage-class=CRawDiskImage -owners off -noverify -noautoopen 2>/dev/null); then
    rm -rf "$tmp"
    return 1
  fi
  if ! dir=$(guest_ssh_dir "$attach" "$tmp/mounts"); then
    detach_guest_disk "$disk"
    rm -rf "$tmp"
    return 2
  fi
  if ! cat "$tmp/key" > "$dir/private/etc/ssh/ssh_host_ed25519_key" || \
    ! cat "$tmp/key.pub" > "$dir/private/etc/ssh/ssh_host_ed25519_key.pub"; then
    detach_guest_disk "$disk"
    rm -rf "$tmp"
    return 1
  fi
  detach_guest_disk "$disk"
  if ! cp "$tmp/key.pub" "$pub"; then
    rm -rf "$tmp"
    return 1
  fi
  rm -rf "$tmp"
  return 0
}

pin_host_key() {
  ip=$1
  pub=$AD_SANDBOX_DIR/vm/host_key.pub
  [ -s "$pub" ] || return 0
  printf '%s %s\n' "$ip" "$(awk 'NR == 1 { print $1, $2 }' "$pub")" > "$AD_SANDBOX_DIR/known_hosts" || return 1
  chmod 0600 "$AD_SANDBOX_DIR/known_hosts"
}

# A guest that needs a person is only reachable through its window, so a headless one must be restarted.
window_is_open() {
  ps -ww -o command= -p "$(cat "$VM_DIR/vmhost.pid" 2>/dev/null)" 2>/dev/null | grep -q -- '--window'
}

needs_setup() {
  if window_is_open; then
    emit_ok needs_setup "$1" "agent-desktop sandbox status $2"
  else
    rm -f "$AD_SANDBOX_DIR/setup_done"
    emit_ok needs_setup "$1 The sandbox is running without a window; stop it and start it again to open one." "agent-desktop sandbox stop $2"
  fi
}

setup_window() {
  needs_setup "Sandbox $1 is running in a setup window; enable Remote Login, run guest-setup.sh in Terminal, and grant Accessibility permission." "$1"
}

ssh_refused_too_long() {
  [ -f "$VM_DIR/ssh_refused" ] || printf '' > "$VM_DIR/ssh_refused" 2>/dev/null || return 1
  refused_since=$(file_mtime "$VM_DIR/ssh_refused")
  case $refused_since in ''|*[!0-9]*|0) return 1 ;; esac
  [ $(( $(date +%s) - refused_since )) -ge 120 ]
}

guest_not_answering() {
  if [ -f "$AD_SANDBOX_DIR/setup_done" ]; then
    if [ -n "${2:-}" ] && ssh_refused_too_long; then
      needs_setup "Sandbox $1 has had a network address for over two minutes, but Remote Login refuses the guest user $2. Check in the guest that Remote Login is on and that this account exists." "$1"
      return 0
    fi
    emit_ok working "Sandbox $1 is running and the guest is still starting." "agent-desktop sandbox status $1"
  else
    needs_setup "Remote Login is unreachable, agent-desktop is not installed, and Accessibility is not granted in the guest." "$1"
  fi
}

missing_guest_binary() {
  next="agent-desktop sandbox status $1"
  [ ! -f "$AD_SANDBOX_DIR/setup_done" ] || next="agent-desktop sandbox exec $1 -- version"
  emit_ok needs_setup "agent-desktop is not installed in the guest, so Accessibility permission cannot yet be checked." "$next"
}

vmhost_missing() {
  emit_fail ACTION_FAILED "The macOS VM host is not installed." "Run agent-desktop sandbox install macos."
}

check_guest_setup() {
  name=$1
  if ! user=$(guest_user); then
    needs_setup "Sandbox $name needs a guest user before SSH setup can continue." "$name"
    return 0
  fi
  if [ ! -x "$VMHOST" ]; then
    vmhost_missing
    return 1
  fi
  ip_json=$("$VMHOST" ip "$VM_DIR" 2>/dev/null || true)
  ip=$(printf '%s' "$ip_json" | sed -n 's/.*"ip"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
  if [ -z "$ip" ]; then
    guest_not_answering "$name"
    return 0
  fi
  if ! pin_host_key "$ip"; then
    emit_fail ACTION_FAILED "The SSH host key for sandbox $name could not be saved." "Run agent-desktop sandbox status $name again after checking directory permissions."
    return 1
  fi
  target=$user@$ip
  probe_status=0
  permissions=$(guest_transport ssh "$target" 'if [ ! -x ~/.local/bin/agent-desktop ]; then exit 3; fi; ~/.local/bin/agent-desktop permissions' 2>/dev/null) || probe_status=$?
  if [ "$probe_status" -eq 255 ]; then
    guest_not_answering "$name" "$user"
    return 0
  fi
  rm -f "$VM_DIR/ssh_refused"
  if [ "$probe_status" -eq 3 ]; then
    missing_guest_binary "$name"
    return 0
  fi
  compact=$(printf '%s' "$permissions" | tr -d '[:space:]')
  case $compact in
    *'"accessibility":{"state":"granted"'*)
      if [ ! -f "$AD_SANDBOX_DIR/setup_done" ] && ! printf '' > "$AD_SANDBOX_DIR/setup_done"; then
        emit_fail ACTION_FAILED "The setup record for sandbox $name could not be saved." "Run agent-desktop sandbox status $name again after checking directory permissions."
        return 1
      fi
      emit_ok running "Sandbox $name is running and guest setup is complete."
      ;;
    *) needs_setup "Accessibility permission is not granted for agent-desktop in the guest." "$name" ;;
  esac
}

spawn_vmhost() {
  mode=$1
  if [ ! -x "$VMHOST" ] || ! command -v nohup >/dev/null 2>&1 || \
    ! : >> "$VM_DIR/vmhost.log" || ! rm -f "$VM_DIR/vmhost.started" "$VM_DIR/ssh_refused"; then
    return 1
  fi
  if [ "$mode" = window ]; then
    nohup "$VMHOST" run "$VM_DIR" --window ${AD_SET_TITLE:+"$AD_SET_TITLE"} >> "$VM_DIR/vmhost.log" 2>&1 < /dev/null &
  else
    nohup "$VMHOST" run "$VM_DIR" >> "$VM_DIR/vmhost.log" 2>&1 < /dev/null &
  fi
  launched_pid=$!
  attempts=0
  while [ "$attempts" -lt 300 ]; do
    if [ "$(sandbox_phase "$AD_SANDBOX_DIR")" = running ]; then return 0; fi
    kill -0 "$launched_pid" 2>/dev/null || return 1
    sleep 0.1
    attempts=$((attempts + 1))
  done
  kill -TERM "$launched_pid" 2>/dev/null || true
  return 1
}

do_start() {
  name=$1
  reject_unknown_settings "window title" || return 1
  case ${AD_SET_WINDOW:-off} in
    on|off) ;;
    *)
      emit_fail INVALID_ARGS "The window setting is on or off." "Run agent-desktop sandbox start $name --window."
      return 1
      ;;
  esac
  mode=headless
  if [ ! -f "$AD_SANDBOX_DIR/setup_done" ] || [ "${AD_SET_WINDOW:-off}" = on ] || [ -n "${AD_SET_TITLE:-}" ]; then mode=window; fi
  phase=$(sandbox_phase "$AD_SANDBOX_DIR")
  if report_phase "$name" "$phase"; then return 0; fi
  if [ "$phase" = running ] && [ "$mode" = window ] && ! window_is_open; then
    emit_fail INVALID_ARGS "Sandbox $name is already running without a window." "Run agent-desktop sandbox stop $name, then start it again with --window."
    return 1
  fi
  if [ "$phase" = stopped ]; then
    if ! host_runtime_supported; then return 0; fi
    if ! provider_ready; then
      vmhost_missing
      return 1
    fi
    rotate_status=0
    rotate_host_key || rotate_status=$?
    if [ "$rotate_status" -ne 0 ] && [ "$rotate_status" -ne 2 ]; then
      emit_fail ACTION_FAILED "Sandbox $name could not be given its own SSH host key." "Run agent-desktop sandbox status $name; a guest disk that cannot be opened cannot be started safely."
      return 1
    fi
    if ! spawn_vmhost "$mode"; then
      emit_fail ACTION_FAILED "The VM host for sandbox $name did not start; see $VM_DIR/vmhost.log." "Run agent-desktop sandbox status $name."
      return 1
    fi
  fi
  if [ -f "$AD_SANDBOX_DIR/setup_done" ]; then
    check_guest_setup "$name"
  else
    setup_window "$name"
  fi
}

do_stop() {
  name=$1
  phase=$(sandbox_phase "$AD_SANDBOX_DIR")
  case $phase in
    installing)
      emit_fail ACTION_FAILED "Sandbox $name is still installing, so there is nothing to stop." "Run agent-desktop sandbox status $name."
      return 1
      ;;
    failed) report_phase "$name" "$phase"; return 0 ;;
    stopped) emit_ok stopped "Sandbox $name is already stopped."; return 0 ;;
  esac
  stop_pid=$(cat "$VM_DIR/vmhost.pid" 2>/dev/null || true)
  kill -TERM "$stop_pid" 2>/dev/null || true
  attempts=0
  while vmhost_is_live "$VM_DIR"; do
    if [ "$attempts" -ge 120 ]; then
      emit_fail TIMEOUT "Sandbox $name did not stop within 24 seconds." "Run agent-desktop sandbox status $name."
      return 1
    fi
    sleep 0.2
    attempts=$((attempts + 1))
  done
  emit_ok stopped "Sandbox $name is stopped."
}

do_status() {
  name=$1
  phase=$(sandbox_phase "$AD_SANDBOX_DIR")
  if report_phase "$name" "$phase"; then return 0; fi
  if [ "$phase" = stopped ]; then
    emit_ok stopped "Sandbox $name is stopped." "agent-desktop sandbox start $name"
    return 0
  fi
  check_guest_setup "$name"
}

clear_share_permissions() {
  [ -d "$VM_DIR/share" ] || return 0
  chmod -R u+rwX "$VM_DIR/share" 2>/dev/null || true
}

do_rm() {
  name=$1
  phase=$(sandbox_phase "$AD_SANDBOX_DIR")
  case $phase in
    installing)
      emit_fail ACTION_FAILED "Sandbox $name is still installing, so it was not removed." "Run \`agent-desktop sandbox status $name\` and remove it after the install finishes or fails."
      return 1
      ;;
    starting|running)
      emit_fail INVALID_ARGS "Sandbox $name is running, so it was not removed." "Run agent-desktop sandbox stop $name first."
      return 1
      ;;
  esac
  clear_share_permissions
  if ! rm -rf "$VM_DIR"; then
    emit_fail ACTION_FAILED "The VM files for sandbox $name could not be removed." "Run agent-desktop sandbox status $name."
    return 1
  fi
  emit_ok stopped "The VM files for sandbox $name were removed."
}

shell_quote() {
  printf "'"
  printf '%s' "$1" | sed "s/'/'\\\\''/g"
  printf "'"
}

sync_guest_file() {
  target=$1
  source_file=$2
  guest_name=$3
  host_hash=$4
  guest_hash=$5
  if [ "$host_hash" = "$guest_hash" ]; then return 0; fi
  staged="~/.local/bin/$guest_name.$$.new"
  if ! guest_transport ssh "$target" 'mkdir -p ~/.local/bin' >/dev/null || \
    ! guest_transport scp "$source_file" "$target:$staged" >/dev/null || \
    ! guest_transport ssh "$target" "chmod 755 $staged && mv -f $staged ~/.local/bin/$guest_name" >/dev/null; then
    guest_transport ssh "$target" "rm -f $staged" >/dev/null 2>&1 || true
    return 1
  fi
}

remote_login_failure() {
  exec_failure "Remote Login could not be reached in sandbox $1." "Run agent-desktop sandbox status $1."
}

exec_failure() {
  emit_fail ACTION_FAILED "$1" "$2"
  exit 125
}

do_exec() {
  name=$1
  shift
  if [ "${1:-}" != -- ]; then
    exec_failure "The exec verb needs an argument separator." "Run agent-desktop sandbox exec $name -- <args>."
  fi
  shift
  phase=$(sandbox_phase "$AD_SANDBOX_DIR")
  if [ "$phase" != running ]; then
    exec_failure "Sandbox $name is $phase, not running." "Run agent-desktop sandbox status $name."
  fi
  if [ ! -f "$AD_SANDBOX_DIR/setup_done" ]; then
    exec_failure "Sandbox $name still needs guest setup." "Run agent-desktop sandbox status $name."
  fi
  if [ ! -f "$AD_BINARY" ]; then
    exec_failure "The host agent-desktop binary is missing." "Rebuild agent-desktop, then run sandbox exec $name again."
  fi
  if ! require_host_package "Rebuild agent-desktop with its helper, then run sandbox exec $name again."; then
    exit 125
  fi
  user=$(guest_user || true)
  if [ -z "$user" ]; then
    exec_failure "Sandbox $name has no configured guest user." "Run agent-desktop sandbox status $name."
  fi
  if [ ! -x "$VMHOST" ]; then
    vmhost_missing
    exit 125
  fi
  ip_json=$("$VMHOST" ip "$VM_DIR" 2>/dev/null || true)
  ip=$(printf '%s' "$ip_json" | sed -n 's/.*"ip"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
  if [ -z "$ip" ]; then
    exec_failure "Sandbox $name has no reachable guest address yet." "Run agent-desktop sandbox status $name."
  fi
  if ! pin_host_key "$ip"; then
    exec_failure "The SSH host key for sandbox $name could not be saved." "Run agent-desktop sandbox status $name again after checking directory permissions."
  fi
  if ! host_hash_line=$(shasum -a 1 "$AD_BINARY" 2>/dev/null); then
    exec_failure "The host agent-desktop binary could not be hashed." "Rebuild agent-desktop, then run sandbox exec $name again."
  fi
  host_helper=$(host_helper_path)
  if ! host_helper_hash_line=$(shasum -a 1 "$host_helper" 2>/dev/null); then
    exec_failure "The host $HELPER_NAME binary could not be hashed." "Rebuild agent-desktop with its helper, then run sandbox exec $name again."
  fi
  host_hash=$(printf '%s\n' "$host_hash_line" | awk '{ print tolower($1) }')
  host_helper_hash=$(printf '%s\n' "$host_helper_hash_line" | awk '{ print tolower($1) }')
  target=$user@$ip
  probe_status=0
  guest_hash_line=$(guest_transport ssh "$target" 'shasum -a 1 ~/.local/bin/agent-desktop' 2>/dev/null) || probe_status=$?
  if [ "$probe_status" -eq 255 ]; then remote_login_failure "$name"; fi
  guest_hash=$(printf '%s\n' "$guest_hash_line" | awk '{ print tolower($1) }')
  probe_status=0
  guest_helper_hash_line=$(guest_transport ssh "$target" "shasum -a 1 ~/.local/bin/$HELPER_NAME" 2>/dev/null) || probe_status=$?
  if [ "$probe_status" -eq 255 ]; then remote_login_failure "$name"; fi
  guest_helper_hash=$(printf '%s\n' "$guest_helper_hash_line" | awk '{ print tolower($1) }')
  if ! sync_guest_file "$target" "$AD_BINARY" agent-desktop "$host_hash" "$guest_hash"; then
    exec_failure "agent-desktop could not be copied into sandbox $name." "Run agent-desktop sandbox status $name."
  fi
  if ! sync_guest_file "$target" "$host_helper" "$HELPER_NAME" "$host_helper_hash" "$guest_helper_hash"; then
    exec_failure "The macOS clipboard helper could not be copied into sandbox $name." "Run agent-desktop sandbox status $name."
  fi
  exec_marker="AD_EXEC_EXIT_$$"
  remote_command='~/.local/bin/agent-desktop'
  for argument do
    remote_command="$remote_command $(shell_quote "$argument")"
  done
  remote_command="$remote_command; ad_status=\$?; printf '%s:%s\\n' '$exec_marker' \"\$ad_status\" >&2; exit \"\$ad_status\""
  if ! exec_stderr=$(mktemp "$AD_SANDBOX_DIR/exec-stderr.XXXXXX"); then
    exec_failure "The exec output file for sandbox $name could not be created." "Run agent-desktop sandbox status $name."
  fi
  trap 'rm -f "$exec_stderr" "$exec_stderr.status"' EXIT
  # The guest controls how much it writes to stderr, so only the last 64 KiB ever reaches the host disk.
  exec 3>&1
  {
    ssh_status=0
    guest_transport ssh "$target" "$remote_command" 2>&1 >&3 || ssh_status=$?
    printf '%s\n' "$ssh_status" > "$exec_stderr.status"
  } | tail -c 65536 > "$exec_stderr"
  exec 3>&-
  ssh_status=$(cat "$exec_stderr.status" 2>/dev/null || printf '255\n')
  guest_status=$(sed -n "s/^.*$exec_marker:\([0-9][0-9]*\)\$/\1/p" "$exec_stderr" | tail -n 1)
  sed -e "/^$exec_marker:[0-9][0-9]*\$/d" -e "s/$exec_marker:[0-9][0-9]*\$//" "$exec_stderr" >&2
  if [ "$ssh_status" -eq 255 ] && [ -z "$guest_status" ]; then
    remote_login_failure "$name"
  fi
  return "${guest_status:-$ssh_status}"
}

require_provider_environment
if [ "$verb" = __install_job ]; then
  require_sandbox_environment
  do_install_job "$@"
  exit $?
fi

case $verb in
  check) do_check ;;
  install) do_install || exit $? ;;
  create|start|stop|status|rm|exec)
    require_sandbox_environment
    if ! valid_name "${1:-}"; then
      emit_fail INVALID_ARGS "The $verb verb needs a valid sandbox name." "Run agent-desktop sandbox $verb <name>."
      [ "$verb" != exec ] || exit 125
      exit 1
    fi
    "do_$verb" "$@" || exit $?
    ;;
  *)
    emit_fail INVALID_ARGS "The verb is not part of the macOS VM provider contract." "Run an agent-desktop sandbox command."
    exit 1
    ;;
esac
