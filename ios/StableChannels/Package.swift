// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "StableChannels",
    platforms: [
        .iOS(.v17)
    ],
    dependencies: [
        .package(url: "https://github.com/toneloc/ldk-node.git", exact: "0.7.5"),
        .package(url: "https://github.com/kishikawakatsumi/KeychainAccess.git", from: "4.2.2"),
        .package(url: "https://github.com/twostraws/CodeScanner.git", from: "2.5.0"),
        .package(url: "https://github.com/dagronf/QRCode.git", exact: "28.0.2")
    ],
    targets: [
        .executableTarget(
            name: "StableChannels",
            dependencies: [
                .product(name: "LDKNode", package: "ldk-node"),
                "KeychainAccess",
                "CodeScanner",
                .product(name: "QRCode", package: "QRCode")
            ],
            path: "StableChannels"
        ),
        .testTarget(
            name: "StableChannelsTests",
            dependencies: ["StableChannels"],
            path: "StableChannelsTests"
        )
    ]
)
