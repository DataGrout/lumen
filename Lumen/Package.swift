// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "Lumen",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "Lumen",
            path: "Sources",
            exclude: ["Info.plist"]
        ),
    ]
)
