#!/usr/bin/env bash
BASE="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
PID_FILE="$BASE/llm-audit.pid"
echo "$BASE"

if [[ ! -f "$PID_FILE" ]]; then
  echo "llm-audit is not running (pid file not found)"
  exit 0
fi

PID="$(cat "$PID_FILE")"
if kill -0 "$PID" 2>/dev/null; then
  kill -TERM "$PID"
  for _ in {1..50}; do
    if ! kill -0 "$PID" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
fi
rm -f "$PID_FILE"
echo "llm-audit stopped"
