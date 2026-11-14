#if canImport(ActivityKit)
import ActivityKit
import Foundation

/// One account-wide Live Activity for all running agents (docs/PUSH.md,
/// "Many agents"). Shared by the app (JcodeBridge) and the widget (JcodeLive).
/// The Worker pushes ContentState as JSON, so keys and types must match
/// `jcode_mobile_core::agents::AgentsState` exactly. Times are Unix seconds.
@available(iOS 16.1, *)
public struct JcodeAgentsAttributes: ActivityAttributes {
    public struct Item: Codable, Hashable {
        public var id: String
        public var title: String
        /// "running", "waiting" or "error".
        public var state: String
        public var detail: String
        /// Where it runs ("Cloud", "XPS 14"). Older hosts omit it.
        public var origin: String?
        /// Unix seconds the current state began. Older hosts omit it.
        public var since: Int?

        public var sinceDate: Date? {
            guard let since, since > 0 else { return nil }
            return Date(timeIntervalSince1970: TimeInterval(since))
        }
    }

    public struct ContentState: Codable, Hashable {
        public var running: Int
        public var waiting: Int
        public var errored: Int
        public var items: [Item]
        public var updatedAt: Int

        public var total: Int { running + waiting + errored }
    }

    public var title: String

    public init(title: String) {
        self.title = title
    }
}

public enum JcodeActivityText {
    public static func label(_ state: String) -> String {
        switch state {
        case "running": return "Running"
        case "waiting": return "Needs input"
        case "error": return "Failed"
        default: return "Idle"
        }
    }

    public static func symbol(_ state: String) -> String {
        switch state {
        case "running": return "hammer"
        case "waiting": return "questionmark.bubble"
        case "error": return "exclamationmark.triangle"
        default: return "checkmark.circle"
        }
    }

    /// Most urgent state present, for the compact island icon.
    @available(iOS 16.1, *)
    public static func top(_ s: JcodeAgentsAttributes.ContentState) -> String {
        if s.waiting > 0 { return "waiting" }
        if s.errored > 0 { return "error" }
        if s.running > 0 { return "running" }
        return "idle"
    }

    @available(iOS 16.1, *)
    public static func headline(_ s: JcodeAgentsAttributes.ContentState) -> String {
        var parts: [String] = []
        if s.waiting > 0 { parts.append("\(s.waiting) need\(s.waiting == 1 ? "s" : "") input") }
        if s.errored > 0 { parts.append("\(s.errored) failed") }
        if s.running > 0 { parts.append("\(s.running) running") }
        return parts.isEmpty ? "All agents idle" : parts.joined(separator: ", ")
    }
}
#endif
