#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# ---------------------------------------------------------------------------
# Usage: ./publish.sh <version>
# Example: ./publish.sh 0.1.6
# ---------------------------------------------------------------------------

if [ -z "${1:-}" ]; then
  echo "Usage: $0 <version>"
  echo "Example: $0 0.1.6"
  exit 1
fi

VERSION="$1"

echo "==> Bumping versions to $VERSION"

# Root binding package
node -e "
  const fs = require('fs');
  const path = 'crates/astro_napi/package.json';
  const pkg = JSON.parse(fs.readFileSync(path, 'utf8'));
  pkg.version = '$VERSION';
  for (const key of Object.keys(pkg.optionalDependencies ?? {})) {
    pkg.optionalDependencies[key] = '$VERSION';
  }
  fs.writeFileSync(path, JSON.stringify(pkg, null, 2) + '\n');
  console.log('  bumped', path);
"

# Main compiler package
node -e "
  const fs = require('fs');
  const path = 'packages/compiler/package.json';
  const pkg = JSON.parse(fs.readFileSync(path, 'utf8'));
  pkg.version = '$VERSION';
  fs.writeFileSync(path, JSON.stringify(pkg, null, 2) + '\n');
  console.log('  bumped', path);
"

# Platform npm packages (if they exist)
for dir in crates/astro_napi/npm/*/; do
  if [ -f "${dir}package.json" ]; then
    node -e "
      const fs = require('fs');
      const path = '${dir}package.json';
      const pkg = JSON.parse(fs.readFileSync(path, 'utf8'));
      pkg.version = '$VERSION';
      fs.writeFileSync(path, JSON.stringify(pkg, null, 2) + '\n');
      console.log('  bumped', path);
    "
  fi
done

echo ""
echo "==> Installing dependencies"
pnpm install

echo ""
echo "==> Building Linux binaries (cross-compile via zigbuild)"
TARGETS=(
  "x86_64-unknown-linux-gnu"
  "x86_64-unknown-linux-musl"
  "aarch64-unknown-linux-gnu"
  "aarch64-unknown-linux-musl"
)

OUTPUT_DIRS=(
  "npm/linux-x64-gnu"
  "npm/linux-x64-musl"
  "npm/linux-arm64-gnu"
  "npm/linux-arm64-musl"
)

NODE_FILE_NAMES=(
  "astro.linux-x64-gnu.node"
  "astro.linux-x64-musl.node"
  "astro.linux-arm64-gnu.node"
  "astro.linux-arm64-musl.node"
)

cd crates/astro_napi

for i in "${!TARGETS[@]}"; do
  TARGET="${TARGETS[$i]}"
  OUTPUT_DIR="${OUTPUT_DIRS[$i]}"
  NODE_FILE="${NODE_FILE_NAMES[$i]}"

  echo ""
  echo "  Building $TARGET..."
  pnpm napi build --esm --release --target "$TARGET" --cross-compile --output-dir "$OUTPUT_DIR"

  # Rename astro.node -> astro.<platform>.node if needed
  if [ -f "${OUTPUT_DIR}/astro.node" ] && [ ! -f "${OUTPUT_DIR}/${NODE_FILE}" ]; then
    mv "${OUTPUT_DIR}/astro.node" "${OUTPUT_DIR}/${NODE_FILE}"
    echo "  Renamed astro.node -> ${NODE_FILE}"
  fi
done

echo ""
echo "==> Building TypeScript package"
cd "$SCRIPT_DIR"
pnpm --filter astro-compiler-rs run build

echo ""
echo "==> Publishing platform binary packages"
cd crates/astro_napi
for dir in npm/*/; do
  if [ -f "${dir}package.json" ]; then
    echo "  Publishing ${dir}..."
    npm publish "./${dir}" --access public
  fi
done

echo ""
echo "==> Publishing astro-compiler-binding"
cd "$SCRIPT_DIR"
pnpm --filter astro-compiler-binding publish --no-git-checks

echo ""
echo "==> Publishing astro-compiler-rs"
pnpm --filter astro-compiler-rs publish --no-git-checks

echo ""
echo "Done! Published version $VERSION"
