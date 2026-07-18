#!/bin/bash
set -e

# Usage helper
if [ -z "$1" ]; then
  echo "Usage: $0 <version> (e.g., v0.1.0)"
  exit 1
fi

VERSION=$1

echo "=== 1. Ensuring target toolchains are installed ==="
rustup target add aarch64-apple-darwin x86_64-apple-darwin

echo "=== 2. Compiling binaries for multi-architectures ==="
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

echo "=== 3. Packing binaries into archives ==="
mkdir -p dist
tar -czf dist/s7s-mac-arm64.tar.gz -C target/aarch64-apple-darwin/release s7s
tar -czf dist/s7s-mac-amd64.tar.gz -C target/x86_64-apple-darwin/release s7s

echo "=== 4. SHA256 checksums (Required for Homebrew Formula) ==="
ARM_HASH=$(shasum -a 256 dist/s7s-mac-arm64.tar.gz | awk '{print $1}')
AMD_HASH=$(shasum -a 256 dist/s7s-mac-amd64.tar.gz | awk '{print $1}')

echo "--------------------------------------------------"
echo "ARM64 SHA256: $ARM_HASH"
echo "AMD64 SHA256: $AMD_HASH"
echo "--------------------------------------------------"

echo "=== 5. Creating Git tag and pushing ==="
git tag "$VERSION"
git push origin "$VERSION"

echo "=== 6. Creating GitHub Release and uploading assets ==="
gh release create "$VERSION" dist/s7s-mac-arm64.tar.gz dist/s7s-mac-amd64.tar.gz \
  --title "$VERSION" \
  --notes "Release $VERSION"

echo "=== Release task-124 finished successfully ==="
echo "Copy the SHA256 hashes above to update your Homebrew Formula."
