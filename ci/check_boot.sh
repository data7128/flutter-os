#!/usr/bin/env bash
#
# ci/check_boot.sh — QEMU boot test for flutter-os.
#
# Runs the BIOS disk image in QEMU (software-emulated, no KVM) and checks
# the serial output for subsystem boot markers. All required markers must
# be present for the test to pass.
#
# Usage: ./ci/check_boot.sh [image_path]
#
# ── CI limitations ──────────────────────────────────────────────────────
# GitHub Actions runners have NO hardware virtualization (no KVM/VT-x).
# QEMU falls back to TCG software emulation, which is significantly slower.
# This script can ONLY detect boot-log markers on the serial console.
# It CANNOT test:
#   - Graphics / framebuffer visual output
#   - Keyboard interaction
#   - Real-time behavior / timer accuracy
# ────────────────────────────────────────────────────────────────────────

set -euo pipefail

IMAGE="${1:-target/aeros-os-bios.img}"
TIMEOUT_SEC=30

OUTPUT_FILE=$(mktemp)

echo "═══════════════════════════════════════════════"
echo "  flutter-os QEMU Boot Test"
echo "═══════════════════════════════════════════════"
echo "  Image:   ${IMAGE}"
echo "  Timeout: ${TIMEOUT_SEC}s"
echo "  Mode:    TCG software emulation (no KVM)"
echo ""

if [ ! -f "${IMAGE}" ]; then
    echo "ERROR: Disk image not found at ${IMAGE}"
    exit 1
fi

# Run QEMU with timeout to prevent CI hang.
# -serial stdio  : serial port → stdout (kernel boot logs)
# -display none  : no GUI window (headless CI)
# -no-reboot     : triple fault → exit instead of reboot loop
# -monitor none  : disable QEMU monitor (non-interactive)
echo "[qemu] starting..."
timeout "${TIMEOUT_SEC}" qemu-system-x86_64 \
    -drive format=raw,file="${IMAGE}" \
    -serial stdio \
    -display none \
    -no-reboot \
    -monitor none \
    2>&1 | tee "${OUTPUT_FILE}" || true

echo ""
echo "═══════════════════════════════════════════════"
echo "  Boot Log Analysis"
echo "═══════════════════════════════════════════════"

ALL_OK=true

# Required subsystem markers.
# [OK] = subsystem initialised successfully.
# [PENDING] = subsystem not yet implemented (kernel still booted to this point).
# Missing marker = kernel crashed before reaching this subsystem → FAIL.
for marker in "GDT" "IDT" "PIC" "HEAP" "KEYBOARD" "GRAPHICS" "TIME" "SYSCALLS" "USERMODE" "SCHEDULER" "SIGNAL" "FORK_EXEC"; do
    if grep -q "\[OK\] ${marker}\|\[PENDING\] ${marker}\|\[WARN\] ${marker}" "${OUTPUT_FILE}"; then
        echo "  ✓ ${marker} marker found"
    else
        echo "  ✗ ${marker} marker NOT found"
        ALL_OK=false
    fi
done

echo ""
rm -f "${OUTPUT_FILE}"

if [ "${ALL_OK}" = "true" ]; then
    echo "═══════════════════════════════════════════════"
    echo "  RESULT: PASS ✓"
    echo "  All subsystem boot markers detected."
    echo "═══════════════════════════════════════════════"
    exit 0
else
    echo "═══════════════════════════════════════════════"
    echo "  RESULT: FAIL ✗"
    echo "  Missing subsystem boot markers."
    echo ""
    echo "  Note: CI can only detect boot-log markers."
    echo "  Cannot test graphics, keyboard interaction,"
    echo "  or real-time behavior."
    echo "═══════════════════════════════════════════════"
    exit 1
fi
