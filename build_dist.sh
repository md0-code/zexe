#!/bin/bash
set -e

echo "Building Release Binaries..."

# Ensure we are in the script's directory
cd "$(dirname "$0")"

# Build Runner
echo "Building zexe-runner..."
cd zexe-runner
cargo build --release
cd ..

# Build Bundler
echo "Building zexe-bundler..."
cd zexe-bundler
cargo build --release
cd ..

# Create Dist
mkdir -p dist

# Copy Binaries
echo "Copying binaries to dist..."
cp "zexe-runner/target/release/zexe-runner" "dist/zexe-runner"
cp "zexe-bundler/target/release/zexe-bundler" "dist/zexe-bundler"

# Compress Binaries with UPX if available
if command -v upx > /dev/null; then
    echo "Compressing binaries with UPX..."
    upx --best "dist/zexe-runner" "dist/zexe-bundler"
else
    echo "UPX not found, skipping compression. (Recommended for smaller binaries)"
fi

# Re-bundle test to verify packing
echo "Re-bundling test for verification..."
if [ -f "test.z80" ]; then
    ./dist/zexe-bundler test.z80 --output dist/test --runner dist/zexe-runner
fi

echo ""
echo "Build Complete!"
echo "Binaries are in the 'dist' folder."
echo ""
