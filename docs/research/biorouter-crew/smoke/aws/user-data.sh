#!/bin/bash
set -euo pipefail
umask 077
mkdir -p /opt/crew-smoke /var/lib/crew-smoke
groupadd crew-smoke
for username in crew_alice crew_bob; do
  useradd -m -G crew-smoke "$username"
  install -d -m 700 -o "$username" -g "$username" "/home/$username/.ssh"
  install -m 600 -o "$username" -g "$username" /home/ec2-user/.ssh/authorized_keys "/home/$username/.ssh/authorized_keys"
done
base64 -d > /opt/crew-smoke/broker.py <<'CREW_BROKER'
@@BROKER@@
CREW_BROKER
base64 -d > /opt/crew-smoke/client.py <<'CREW_CLIENT'
@@CLIENT@@
CREW_CLIENT
chmod 755 /opt/crew-smoke /opt/crew-smoke/*.py
cat > /etc/systemd/system/crew-smoke.service <<'CREW_UNIT'
[Unit]
Description=Synthetic Crew feasibility broker
After=network.target
[Service]
ExecStart=/usr/bin/python3 /opt/crew-smoke/broker.py
Restart=on-failure
UMask=0077
[Install]
WantedBy=multi-user.target
CREW_UNIT
systemctl daemon-reload
systemctl enable --now crew-smoke.service
for item in gate-a:2222 gate-b:2223 target:2224; do
  name=${item%:*}
  port=${item#*:}
  ssh-keygen -q -t ed25519 -N '' -f "/opt/crew-smoke/$name-key"
  cat > "/opt/crew-smoke/$name-sshd.conf" <<CREW_SSHD
Port $port
ListenAddress 127.0.0.1
HostKey /opt/crew-smoke/$name-key
PidFile /run/crew-$name-sshd.pid
AuthorizedKeysFile .ssh/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
UsePAM yes
AllowTcpForwarding yes
AllowAgentForwarding no
X11Forwarding no
Subsystem sftp internal-sftp
CREW_SSHD
  /usr/sbin/sshd -f "/opt/crew-smoke/$name-sshd.conf"
done
echo "CREW_HOSTKEY entry $(cat /etc/ssh/ssh_host_ed25519_key.pub)" > /dev/console
for name in gate-a gate-b target; do
  echo "CREW_HOSTKEY $name $(cat /opt/crew-smoke/$name-key.pub)" > /dev/console
done
echo 'CREW_READY' > /dev/console
