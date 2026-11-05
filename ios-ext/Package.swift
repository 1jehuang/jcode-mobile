// swift-tools-version:5.9
// iOS extensions for jcode-mobile, cross-compiled from Linux with the Darwin
// Swift SDK by scripts/ios-ext.sh (called from scripts/ios-bundle.sh):
//
//   JcodeBridge  dynamic library embedded as Frameworks/JcodeBridge.framework.
//                The Rust app dlopens it and calls its @_cdecl functions to
//                start/update/end the Live Activity (ActivityKit is Swift-only).
//   JcodeLive    WidgetKit extension rendering the Live Activity and the
//                Dynamic Island (PlugIns/JcodeLive.appex).
//   JcodeShare   Share extension forwarding shared text/URLs to the app as
//                jcode://new?text=... (PlugIns/JcodeShare.appex).
import PackageDescription

let appex: [LinkerSetting] = [.unsafeFlags(["-Xlinker", "-e", "-Xlinker", "_NSExtensionMain"])]
let appexSwift: [SwiftSetting] = [.unsafeFlags(["-application-extension"])]

let package = Package(
    name: "JcodeExt",
    platforms: [.iOS(.v15)],
    products: [
        .library(name: "JcodeBridge", type: .dynamic, targets: ["JcodeBridge"]),
        .executable(name: "JcodeLive", targets: ["JcodeLive"]),
        .executable(name: "JcodeShare", targets: ["JcodeShare"]),
    ],
    targets: [
        .target(name: "JcodeLiveShared", swiftSettings: appexSwift),
        .target(name: "JcodeBridge", dependencies: ["JcodeLiveShared"]),
        .executableTarget(name: "JcodeLive", dependencies: ["JcodeLiveShared"], swiftSettings: appexSwift + [.unsafeFlags(["-parse-as-library"])], linkerSettings: appex),
        .executableTarget(name: "JcodeShare", swiftSettings: appexSwift + [.unsafeFlags(["-parse-as-library"])], linkerSettings: appex),
    ]
)
