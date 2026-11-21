// swift-tools-version:5.5

import PackageDescription

let package = Package(
  name: "tauri-plugin-carminium-audio",
  platforms: [
    .macOS(.v10_15),
    .iOS(.v15),
  ],
  products: [
    .library(
      name: "tauri-plugin-carminium-audio",
      type: .static,
      targets: ["tauri-plugin-carminium-audio"]
    )
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-carminium-audio",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources"
    )
  ]
)
