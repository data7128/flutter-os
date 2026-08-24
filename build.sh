#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────
#  AeroOS Build Script
#  Builds the kernel, user-mode crates, and bootable BIOS disk image.
# ──────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

export PATH="$HOME/.cargo/bin:$PATH"

echo "═══════════════════════════════════════════════"
echo "  AeroOS Build"
echo "═══════════════════════════════════════════════"

# ── 1. Check all user-mode crates compile ───────────────
echo ""
echo "[1/4] Checking user-mode crates..."
cargo check -p wm --release 2>&1 | tail -1
cargo check -p flutter_shell --release 2>&1 | tail -1
cargo check -p flutter_adapter --release 2>&1 | tail -1
cargo check -p sysutils --release 2>&1 | tail -1
echo "  ✓ All user-mode crates compile"

# ── 2. Build kernel + BIOS disk image ────────────────────
echo ""
echo "[2/4] Building kernel and BIOS disk image..."
cargo build -p aeros-os --release

# ── 3. Copy the disk image to a stable path ──────────────
echo ""
echo "[3/4] Running image builder..."
IMAGE="target/aeros-os-bios.img"
cargo run -p aeros-os --release --quiet

# ── 4. Verify image ──────────────────────────────────────
echo ""
echo "[4/4] Verifying disk image..."
if [ -f "$IMAGE" ]; then
    echo "  ✓ Disk image: $IMAGE ($(du -h "$IMAGE" | cut -f1))"
    echo ""
    echo "═══════════════════════════════════════════════"
    echo "  Build complete!"
    echo ""
    echo "  Run in QEMU:"
    echo "    qemu-system-x86_64 \\"
    echo "      -drive format=raw,file=$IMAGE \\"
    echo "      -serial stdio \\"
    echo "      -device usb-uhci \\"
    echo "      -usbdevice keyboard \\"
    echo "      -usbdevice mouse"
    echo "═══════════════════════════════════════════════"
else
    echo "  ✗ Disk image not found at $IMAGE"
    exit 1
fi
