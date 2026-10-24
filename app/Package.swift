// swift-tools-version:5.9
import Foundation
import PackageDescription

// ProjectStore (Sources/Pixmith/App/Projects.swift) calls the core's pixmith_project_* functions
// (Sources/PixmithCore/pixmith_project.h). PIXMITH_PROJECT_FALLBACK=1 builds the pure-Swift fallback instead.
let projectCore = ProcessInfo.processInfo.environment["PIXMITH_PROJECT_FALLBACK"] == nil

// scripts/package.sh points this at a directory holding the (possibly universal) libpixmith_core.a.
let coreLib = ProcessInfo.processInfo.environment["PIXMITH_CORE_LIB_DIR"] ?? Context.packageDirectory + "/../core/target/release"
let link = LinkerSetting.unsafeFlags(["-L", coreLib])

// All app code lives in the PixmithKit library so the pixmith-qa scenario runner can drive it; Pixmith is only the
// @main entry point. PixmithKit is built with -enable-testing so both executables can `@testable import` it.
let package = Package(
    name: "Pixmith",
    platforms: [.macOS(.v13)],
    targets: [
        .systemLibrary(name: "PixmithCore", path: "Sources/PixmithCore"),
        .target(
            name: "PixmithKit",
            dependencies: ["PixmithCore"],
            path: "Sources/Pixmith",
            swiftSettings: [.unsafeFlags(["-enable-testing"])] + (projectCore ? [.define("PIXMITH_PROJECT_CORE")] : []),
            linkerSettings: [link]
        ),
        .executableTarget(
            name: "Pixmith",
            dependencies: ["PixmithKit"],
            path: "Sources/PixmithMain",
            linkerSettings: [link]
        ),
        .executableTarget(
            name: "pixmith-qa",
            dependencies: ["PixmithKit", "PixmithCore"],
            path: "Sources/PixmithQA",
            linkerSettings: [link]
        ),
    ]
)
