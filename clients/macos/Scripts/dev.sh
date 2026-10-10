#!/bin/bash
set -euo pipefail
# Releash の terminal から実行すると本番の data dir を引き継ぐため、dev の既定に戻す
unset RELEASH_DATA_DIR
cd "$(dirname "$0")/.."
swift run --package-path BuildTools xcodegen generate
xcodebuild -project Releash.xcodeproj -scheme Releash -configuration Debug \
  -destination 'platform=macOS' -derivedDataPath .build/DerivedData \
  -clonedSourcePackagesDirPath .build/SourcePackages CODE_SIGNING_ALLOWED=NO build
open .build/DerivedData/Build/Products/Debug/Releash.app
