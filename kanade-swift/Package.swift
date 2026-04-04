// swift-tools-version: 6.3

import PackageDescription

let package = Package(
    name: "KanadeSwift",
    products: [
        .library(
            name: "KanadeSwift",
            targets: ["KanadeSwift"]
        )
    ],
    targets: [
        .target(
            name: "KanadeSwift"
        ),
        .executableTarget(
            name: "KanadeNativeApp",
            dependencies: ["KanadeSwift"],
            path: "Examples/KanadeNativeApp/Sources"
        ),
        .testTarget(
            name: "KanadeSwiftTests",
            dependencies: ["KanadeSwift"]
        )
    ],
    swiftLanguageModes: [.v6]
)
