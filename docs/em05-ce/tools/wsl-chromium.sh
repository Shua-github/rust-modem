#!/usr/bin/env bash
# Launch the Linux Chromium at the local cli-web shell inside WSLg.
#
# --no-sandbox is needed because WSL usually runs as root; --disable-gpu avoids
# the WSLg GPU process (there is no DRM device on the Windows side).

URL="${1:-http://127.0.0.1:8123/}"
PROFILE="${CHROMIUM_PROFILE:-$HOME/.config/chromium-wsl}"

pkill -x chromium 2>/dev/null
sleep 1

setsid nohup /usr/sbin/chromium \
  --no-sandbox \
  --disable-gpu \
  --disable-dev-shm-usage \
  --no-first-run \
  --no-default-browser-check \
  --user-data-dir="$PROFILE" \
  "$URL" >/tmp/chromium.log 2>&1 </dev/null &

sleep 6
echo "chromium processes: $(pgrep -x chromium | wc -l)"
