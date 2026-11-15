// swift-tools-version: 5.9
// The app's own force close (#456) for iOS. mobile/force-close/build-ios.sh generates the framework
// and the Swift bindings.
import PackageDescription

let package = Package(
    name: "SCForceClose",
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "SCForceClose", targets: ["SCForceClose"])
    ],
    targets: [
        .binaryTarget(name: "sc_force_closeFFI", path: "sc_force_closeFFI.xcframework"),
        .target(name: "SCForceClose", dependencies: ["sc_force_closeFFI"], path: "Sources/SCForceClose")
    ]
)
