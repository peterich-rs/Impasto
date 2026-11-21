#!/usr/bin/env bash
set -euo pipefail

src=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo=$(cd -- "$src/../.." && pwd)
out=${1:-"$repo/target/android-flutter-reference"}
flutter=${FLUTTER:-flutter}
mkdir -p "$out"
out=$(cd -- "$out" && pwd)
if [[ ! -f "$out/android/gradlew" ]]; then
  "$flutter" create --platforms=android --org dev.impasto \
    --project-name market_reference --empty --no-pub "$out"
fi
mkdir -p "$out/assets"
cp "$src/main.dart" "$out/lib/main.dart"
cp "$src/pubspec.yaml" "$out/pubspec.yaml"
node --experimental-strip-types "$src/export-market.ts" > "$out/assets/market.json"
cd "$out"
"$flutter" --version --machine > flutter-version.json
"$flutter" build apk --profile --target-platform android-arm64,android-x64
