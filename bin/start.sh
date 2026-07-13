#!/usr/bin/env bash
BASE="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
PID_FILE="$BASE/llm-audit.pid"
echo "base:$BASE"
cd "$BASE" || exit 1
mkdir -p "$BASE/logs"
if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
  echo "llm-audit is already running (pid: $(cat "$PID_FILE"))"
  exit 1
fi
nohup "$BASE/bin/llm-audit" > "$BASE/logs/llm-audit.log" 2>&1 &
PID=$!
echo "$PID" > "$PID_FILE"
echo "logs: $BASE/logs/llm-audit.log"
echo "pid: $PID"
echo "llm-audit started"
