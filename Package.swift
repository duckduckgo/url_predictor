// swift-tools-version: 5.7
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "URLPredictor",
    platforms: [
        .iOS(.v15),
        .macOS(.v11),
    ],
    products: [
        .library(name: "URLPredictor", targets: ["URLPredictor", "URLPredictorRust"]),
    ],
    targets: [
        .target(name: "URLPredictor", dependencies: ["URLPredictorRust"], path: "apple/Sources/URLPredictor"),
        .binaryTarget(
            name: "URLPredictorRust",
            path: "apple/URLPredictorRust.xcframework"
        ),
        .testTarget(name: "URLPredictorTests", dependencies: ["URLPredictor"], path: "apple/Sources/URLPredictorTests")
    ]
)
