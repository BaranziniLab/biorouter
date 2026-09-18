#!/usr/bin/env bash
set -euo pipefail
# The pinned Bullseye image retains signed snapshot sources after distro EOL.
sed -i 's|^deb http://deb.debian.org|# deb http://deb.debian.org|; s|^# deb http://snapshot.debian.org|deb http://snapshot.debian.org|' /etc/apt/sources.list
printf 'Acquire::Check-Valid-Until "false";\n' > /etc/apt/apt.conf.d/99no-check-valid-until
apt-get update -q
apt-get install -y --no-install-recommends libgtk-3-0 libnss3 libasound2 libgbm1 libxss1 libatk-bridge2.0-0
