// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Nucleo",
    platforms: [.macOS(.v14)],
    products: [.library(name: "Nucleo", targets: ["Nucleo"])],
    targets: [
        .target(
            name: "Nucleo",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [.linkedLibrary("sqlite3")]
        ),
        .testTarget(name: "NucleoTests", dependencies: ["Nucleo"], swiftSettings: [.swiftLanguageMode(.v5)]),
    ]
)
