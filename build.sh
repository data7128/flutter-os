#!/usr/bin/env bash
set -euo pipefail

# AeroOS one-command build script.
# Builds the kernel, the bootable OS image, and (optionally) the Flutter shell.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "═══════════════════════════════════════════════"
echo "  AeroOS Build"
echo "═══════════════════════════════════════════════"

# ── 1. Rust kernel ─────────────────────────────
echo ""
echo "[1/3] Building Rust kernel..."
cd kernel
cargo build
cd ..

# ── 2. Bootable OS image ──────────────────────
echo ""
echo "[2/3] Building bootable OS image..."
cd os
cargo build
cd ..

IMAGE="os/target/x86_64-unknown-none/debug/aeros-os"
if [ -f "$IMAGE" ]; then
    echo "  ✓ Image: $IMAGE ($(du -h "$IMAGE" | cut -f1))"
fi

# ── 3. Flutter shell (optional) ───────────────
if command -v flutter &> /dev/null; then
    echo ""
    echo "[3/3] Building Flutter shell..."
    cd shell
    flutter pub get
    cd ..
    echo "  ✓ Flutter shell ready (run: cd shell && flutter run -d linux)"
else
    echo ""
    echo "[3/3] Flutter SDK not found; skipping shell build."
fi

# ── Summary ───────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════"
echo "  Build complete!"
echo ""
echo "  Run in QEMU:"
echo "    qemu-system-x86_64 \\"
echo "      -drive format=raw,file=$IMAGE \\"
echo "      -serial stdio"
echo "═══════════════════════════════════════════════"
