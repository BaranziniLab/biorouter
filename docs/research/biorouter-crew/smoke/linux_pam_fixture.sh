#!/bin/sh
set -eu

# Test-only setup for a disposable Linux container. The caller supplies the
# password so this harness never stores a reusable secret in the repository.
: "${CREW_PAM_TEST_PASSWORD:?set a synthetic, disposable PAM password}"
CONTAINER="${1:?usage: linux_pam_fixture.sh CONTAINER [USER] }"
USER_NAME="${2:-mfa}"

docker exec "$CONTAINER" sh -eu -c '
  useradd -m -s /bin/bash "$1" 2>/dev/null || true
  printf "%s:%s\n" "$1" "$2" | chpasswd
  install -d -m 700 -o "$1" -g "$1" "/home/$1/.ssh"
  install -m 600 -o "$1" -g "$1" /tmp/client.pub "/home/$1/.ssh/authorized_keys"
  cat >/etc/ssh/sshd_config.d/biorouter-crew-pam.conf <<EOF
Port 22
ListenAddress 0.0.0.0
PermitRootLogin no
AllowUsers $1
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication yes
AuthenticationMethods publickey,keyboard-interactive:pam
UsePAM yes
AllowTcpForwarding yes
X11Forwarding no
LogLevel VERBOSE
EOF
  /usr/sbin/sshd -t -f /etc/ssh/sshd_config
  pkill -TERM sshd 2>/dev/null || true
  /usr/sbin/sshd -D -e -f /etc/ssh/sshd_config >/tmp/sshd.log 2>&1 &
' sh "$USER_NAME" "$CREW_PAM_TEST_PASSWORD"
