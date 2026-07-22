#!/bin/bash
set -e

TOTAL_START=$(date +%s)
SCRIPT_DIR="$(dirname "$0")"
DEPLOY_DIR="/mnt/d/tmp/"
if [ -z "${DEPLOY_DIR}" ]; then
    DEPLOY_DIR="${SCRIPT_DIR}/tmp"
fi
mkdir -p "${DEPLOY_DIR}"

echo "========================================"
echo "Build started at $(date '+%Y-%m-%d %H:%M:%S')"
echo "Deploy dir: ${DEPLOY_DIR}"
echo "========================================"

build_main() {
    echo ""
    echo "--- Building llm-audit ---"
    START=$(date +%s)
    cd "${SCRIPT_DIR}"
    cargo build --release --target x86_64-unknown-linux-musl
    cp target/x86_64-unknown-linux-musl/release/llm-audit "${DEPLOY_DIR}/"
    END=$(date +%s)
    echo "llm-audit done, elapsed: $((END - START))s"
}

build_main

TOTAL_END=$(date +%s)
TOTAL_DURATION=$((TOTAL_END - TOTAL_START))
echo ""
echo "========================================"
echo "Build finished at $(date '+%Y-%m-%d %H:%M:%S'), total elapsed: ${TOTAL_DURATION}s"
echo "========================================"
