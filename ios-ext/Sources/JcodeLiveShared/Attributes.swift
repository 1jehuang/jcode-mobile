#if canImport(ActivityKit)
import ActivityKit
import Foundation

/// Live Activity payload shared by the app (JcodeBridge) and the widget
/// (JcodeLive). Both must agree on this type exactly.
@available(iOS 16.1, *)
public struct JcodeActivityAttributes: ActivityAttributes {
    public struct ContentState: Codable, Hashable {
        /// "thinking", "responding", "tools", "idle", "waiting", "error".
        public var activity: String
        /// Short detail, e.g. the running tool name.
        public var detail: String
        public var startedAt: Date

        public init(activity: String, detail: String, startedAt: Date) {
            self.activity = activity
            self.detail = detail
            self.startedAt = startedAt
        }
    }

    public var title: String
    public var sessionId: String

    public init(title: String, sessionId: String) {
        self.title = title
        self.sessionId = sessionId
    }
}

public enum JcodeActivityText {
    public static func label(_ activity: String) -> String {
        switch activity {
        case "thinking": return "Thinking"
        case "responding": return "Responding"
        case "tools": return "Running tools"
        case "waiting": return "Needs input"
        case "error": return "Error"
        default: return "Idle"
        }
    }

    public static func symbol(_ activity: String) -> String {
        switch activity {
        case "thinking": return "brain"
        case "responding": return "text.bubble"
        case "tools": return "hammer"
        case "waiting": return "questionmark.bubble"
        case "error": return "exclamationmark.triangle"
        default: return "checkmark.circle"
        }
    }
}
#endif
