// C ABI for the Rust app. Loaded with dlopen from
// Frameworks/JcodeBridge.framework, so a missing or older bundle degrades to
// "no Live Activity" instead of failing to launch.
import Foundation
#if canImport(ActivityKit)
import ActivityKit
import JcodeLiveShared
#endif

private func str(_ p: UnsafePointer<CChar>?) -> String {
    p.map { String(cString: $0) } ?? ""
}

/// Starts (or retargets) the Live Activity. Returns 1 on success.
@_cdecl("jcode_live_start")
public func jcode_live_start(_ title: UnsafePointer<CChar>?, _ session: UnsafePointer<CChar>?, _ activity: UnsafePointer<CChar>?, _ detail: UnsafePointer<CChar>?) -> Int32 {
    #if canImport(ActivityKit)
    if #available(iOS 16.2, *) {
        return LiveController.shared.start(title: str(title), session: str(session), activity: str(activity), detail: str(detail)) ? 1 : 0
    }
    #endif
    return 0
}

@_cdecl("jcode_live_update")
public func jcode_live_update(_ activity: UnsafePointer<CChar>?, _ detail: UnsafePointer<CChar>?) {
    #if canImport(ActivityKit)
    if #available(iOS 16.2, *) {
        LiveController.shared.update(activity: str(activity), detail: str(detail))
    }
    #endif
}

@_cdecl("jcode_live_end")
public func jcode_live_end() {
    #if canImport(ActivityKit)
    if #available(iOS 16.2, *) {
        LiveController.shared.end()
    }
    #endif
}

/// 1 if the user allows Live Activities for this app.
@_cdecl("jcode_live_enabled")
public func jcode_live_enabled() -> Int32 {
    #if canImport(ActivityKit)
    if #available(iOS 16.2, *) {
        return ActivityAuthorizationInfo().areActivitiesEnabled ? 1 : 0
    }
    #endif
    return 0
}

/// Donates "New jcode session" and "Ask jcode" as Siri/Shortcuts actions.
/// These are NSUserActivity shortcuts: they appear in the Shortcuts app
/// ("Apps > jcode rs") and Siri Suggestions, and run by continuing the
/// activity, which the app receives as its `url` (jcode://new, jcode://ask).
/// App Intents would be nicer but need Xcode's metadata processor
/// (docs/IOS_EXTENSIONS.md).
private var donated: [NSUserActivity] = []

@_cdecl("jcode_donate_shortcuts")
public func jcode_donate_shortcuts() {
    guard donated.isEmpty else { return }
    let specs: [(String, String, String)] = [
        ("sh.jcode.mobile.new-session", "New jcode session", "jcode://new"),
        ("sh.jcode.mobile.ask", "Ask jcode", "jcode://ask"),
    ]
    for (type, title, url) in specs {
        let a = NSUserActivity(activityType: type)
        a.title = title
        a.userInfo = ["url": url]
        a.isEligibleForSearch = true
        a.isEligibleForPrediction = true
        a.persistentIdentifier = type
        a.becomeCurrent()
        donated.append(a)
    }
}

#if canImport(ActivityKit)
@available(iOS 16.2, *)
final class LiveController {
    static let shared = LiveController()
    private var current: Activity<JcodeActivityAttributes>?
    private var startedAt = Date()

    func start(title: String, session: String, activity: String, detail: String) -> Bool {
        guard ActivityAuthorizationInfo().areActivitiesEnabled else { return false }
        if let cur = current, cur.attributes.sessionId == session, cur.attributes.title == title {
            update(activity: activity, detail: detail)
            return true
        }
        end()
        // Activities left over from a previous launch.
        for a in Activity<JcodeActivityAttributes>.activities {
            Task { await a.end(nil, dismissalPolicy: .immediate) }
        }
        startedAt = Date()
        let state = JcodeActivityAttributes.ContentState(activity: activity, detail: detail, startedAt: startedAt)
        do {
            current = try Activity.request(
                attributes: JcodeActivityAttributes(title: title, sessionId: session),
                content: ActivityContent(state: state, staleDate: nil),
                pushType: nil
            )
            return true
        } catch {
            NSLog("jcode live activity: \(error)")
            return false
        }
    }

    func update(activity: String, detail: String) {
        guard let cur = current else { return }
        let state = JcodeActivityAttributes.ContentState(activity: activity, detail: detail, startedAt: startedAt)
        Task { await cur.update(ActivityContent(state: state, staleDate: nil)) }
    }

    func end() {
        guard let cur = current else { return }
        current = nil
        let state = JcodeActivityAttributes.ContentState(activity: "idle", detail: "", startedAt: startedAt)
        Task { await cur.end(ActivityContent(state: state, staleDate: nil), dismissalPolicy: .after(Date().addingTimeInterval(60))) }
    }
}
#endif
