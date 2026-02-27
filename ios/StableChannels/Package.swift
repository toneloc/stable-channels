// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "StableChannels",
    platforms: [
        .iOS(.v17),
    ],
    dependencies: [
        .package(url: "https://github.com/lightningdevkit/ldk-node.git", exact: "0.7.0"),
        .package(url: "https://github.com/kishikawakatsumi/KeychainAccess.git", from: "4.2.2"),
        .package(url: "https://github.com/twostraws/CodeScanner.git", from: "2.5.0"),
    ],
    targets: [
        .executableTarget(
            name: "StableChannels",
            dependencies: [
                .product(name: "LDKNode", package: "ldk-node"),
                "KeychainAccess",
                "CodeScanner",
            ],
            path: "StableChannels"
        ),
        .testTarget(
            name: "StableChannelsTests",
            dependencies: ["StableChannels"],
            path: "StableChannelsTests"
        ),
    ]
)
