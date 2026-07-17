// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "mdstream_flutter",
    platforms: [
        .iOS(.v13),
    ],
    products: [
        .library(name: "mdstream-flutter", targets: ["mdstream_flutter"]),
    ],
    targets: [
        .binaryTarget(
            name: "MdstreamFFI",
            path: "../MdstreamFFI.xcframework"
        ),
        .target(
            name: "mdstream_flutter",
            dependencies: ["MdstreamFFI"],
            path: "Sources/mdstream_flutter"
        ),
    ]
)
