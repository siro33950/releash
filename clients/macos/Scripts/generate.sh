#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
swift build --package-path clients/macos/BuildTools --product protoc-gen-swift
swift build --package-path clients/macos/BuildTools --product protoc-gen-connect-swift
buf generate --template clients/macos/buf.gen.yaml
buf build -o clients/macos/Generated/client-descriptor.bin --as-file-descriptor-set
