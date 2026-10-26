// swift-tools-version: 6.4
import PackageDescription

let package = Package(
    name: "GDirect",
    platforms: [.macOS(.v15)],
    products: [
        .executable(name: "GDirectApp", targets: ["GDirect"])
    ],
    targets: [
        .executableTarget(name: "GDirect"),
        .testTarget(
            name: "GDirectTests",
            dependencies: ["GDirect"],
            exclude: ["Fixtures"]
        )
    ]
)
