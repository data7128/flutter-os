#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────
#  AeroOS build script
#  Builds the kernel and a bootable BIOS disk image.
# ──────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

export PATH="$HOME/.cargo/bin:$PATH"

echo "═══════════════════════════════════════════════"
echo "  AeroOS Build"
echo "═══════════════════════════════════════════════"

# ── 1. Build kernel + BIOS disk image ──────────────────
echo ""
echo "[1/2] Building kernel and BIOS disk image..."
cargo build -p aeros-os --release

# ── 2. Copy the disk image to a stable path ────────────
echo ""
echo "[2/2] Running image builder..."
IMAGE="target/aeros-os-bios.img"
cargo run -p aeros-os --release --quiet

if [ -f "$IMAGE" ]; then
    echo ""
    echo "  ✓ Disk image: $IMAGE ($(du -h "$IMAGE" | cut -f1))"
    echo ""
    echo "═══════════════════════════════════════════════"
    echo "  Build complete!"
    echo ""
    echo "  Run in QEMU:"
    echo "    qemu-system-x86_64 \\"
    echo "      -drive format=raw,file=$IMAGE \\"
    echo "      -serial stdio"
    echo "═══════════════════════════════════════════════"
else
    echo "  ✗ Disk image not found at $IMAGE"
    exit 1
fi
