#!/bin/sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
key_file="$here/id_ed25519.pub"
binary="$here/agent-desktop"
helper="$here/agent-desktop-macos-helper"
ssh_dir="$HOME/.ssh"
auth_file="$ssh_dir/authorized_keys"
bin_dir="$HOME/.local/bin"
installed="$bin_dir/agent-desktop"
installed_helper="$bin_dir/agent-desktop-macos-helper"

if [ ! -f "$key_file" ]; then
  echo "The public key id_ed25519.pub is not in this folder, so run the sandbox create step on the host again." >&2
  exit 1
fi
if [ ! -f "$binary" ]; then
  echo "The agent-desktop binary is not in this folder, so run the sandbox create step on the host again." >&2
  exit 1
fi
if [ ! -f "$helper" ]; then
  echo "The agent-desktop-macos-helper binary is not in this folder, so run the sandbox create step on the host again." >&2
  exit 1
fi

mkdir -p -m 700 "$ssh_dir"
touch "$auth_file"
chmod 600 "$auth_file"

key=$(cat "$key_file")
if grep -qxF "$key" "$auth_file"; then
  echo "The sandbox key is already in $auth_file."
else
  if [ -s "$auth_file" ] && [ "$(tail -c 1 "$auth_file" | wc -l)" -eq 0 ]; then
    printf '\n' >> "$auth_file"
  fi
  printf '%s\n' "$key" >> "$auth_file"
  echo "Added the sandbox key to $auth_file."
fi

mkdir -p "$bin_dir"
cp "$binary" "$installed.new"
chmod 755 "$installed.new"
mv -f "$installed.new" "$installed"
cp "$helper" "$installed_helper.new"
chmod 755 "$installed_helper.new"
mv -f "$installed_helper.new" "$installed_helper"
echo "Installed agent-desktop and agent-desktop-macos-helper at $bin_dir."

echo "Requesting the macOS permissions agent-desktop needs."
"$installed" permissions --request

echo "Two steps are left, and you do them by hand in System Settings:"
echo "1. Turn on Remote Login in System Settings > General > Sharing."
echo "2. If macOS asks, allow Accessibility for \"sshd-keygen-wrapper\", or find it in System Settings > Privacy & Security > Accessibility and switch it on."
