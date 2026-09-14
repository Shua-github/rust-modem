#!/usr/bin/env bash
# Run an rm-cli command under strace and report the usbfs ioctls it issues.
#
# Usage: repro.sh <rm-cli 参数...>
#   二进制路径优先取 RM_CLI_BIN，否则用仓库里的 target/debug/rm-cli，
#   也可以用第一个参数 --bin <path> 指定。

set -u

bin="${RM_CLI_BIN:-}"
if [ "${1:-}" = "--bin" ]; then
  bin="$2"
  shift 2
fi
if [ -z "$bin" ]; then
  root=$(cd "$(dirname "$0")/../../.." && pwd)
  bin="$root/target/debug/rm-cli"
fi
if [ ! -x "$bin" ]; then
  echo "找不到可执行的 rm-cli：$bin" >&2
  echo "用 --bin <path> 或设置 RM_CLI_BIN 指定" >&2
  exit 2
fi

log=$(mktemp -t rmcli-strace.XXXXXX.log)
echo "=== before $(date +%T) ==="
dmesg 2>/dev/null | tail -1

echo "=== run: $bin $* ==="
timeout 90 strace -f -tt -e trace=ioctl -o "$log" "$bin" "$@"
echo "exit=$?"

echo "=== ioctls ==="
grep -vE "TCGETS|TIOCGWINSZ|TIOCSPGRP|FIONBIO|TCSETS" "$log" | tail -30

echo "=== dmesg ==="
dmesg 2>/dev/null | tail -6
