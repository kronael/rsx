#!/bin/sh
set -eu

# Dev-box janitor: prune tmp/wal files older than 2 hours so the box never
# wedges on disk again (WalWriter's 4h retention is FROZEN and doesn't bound
# this dev dir under maker churn).

WAL_DIR="/home/onvos/sandbox/rsx/tmp/wal"

case "$WAL_DIR" in
  */tmp/wal) ;;
  *) echo "refusing: unexpected path"; exit 1;;
esac

if [ ! -d "$WAL_DIR" ]; then
  echo "$(date '+%b %d %H:%M:%S') INFO prune-dev-wal: $WAL_DIR absent, no-op"
  exit 0
fi

count=$(find "$WAL_DIR" -type f -mmin +120 -print | wc -l)
find "$WAL_DIR" -type f -mmin +120 -delete
find "$WAL_DIR" -mindepth 1 -type d -empty -delete

free_line=$(df -h / | tail -1)
echo "$(date '+%b %d %H:%M:%S') INFO prune-dev-wal: deleted $count files older than 120min from $WAL_DIR; disk: $free_line"
