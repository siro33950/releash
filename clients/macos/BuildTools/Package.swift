// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "BuildTools",
    dependencies: [
        .package(url: "https://github.com/yonaskolb/XcodeGen", exact: "2.44.1"),
        .package(url: "https://github.com/apple/swift-protobuf", exact: "1.38.1"),
        .package(url: "https://github.com/connectrpc/connect-swift", exact: "1.0.0"),
    ]
)
