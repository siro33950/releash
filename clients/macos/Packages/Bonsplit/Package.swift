// swift-tools-version: 5.9

// Vendored from siro33950/bonsplit at 088c284ecdd51721da83e2ae121a97d25881cc38 for tab colors and menus.
import PackageDescription

let package = Package(
    name: "Bonsplit",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .library(
            name: "Bonsplit",
            targets: ["Bonsplit"]
        ),
    ],
    targets: [
        .target(
            name: "Bonsplit",
            dependencies: [],
            path: "Sources/Bonsplit"
        ),
        .testTarget(
            name: "BonsplitTests",
            dependencies: ["Bonsplit"],
            path: "Tests/BonsplitTests"
        ),
    ]
)
