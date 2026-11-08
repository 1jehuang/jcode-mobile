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

/// Called with (kind, hex token). kind 0 = activity update token,
/// 1 = push-to-start token (iOS 17.2+).
public typealias TokenCallback = @convention(c) (Int32, UnsafePointer<CChar>?) -> Void
private var tokenCallback: TokenCallback?

private func hex(_ data: Data) -> String {
    data.map { String(format: "%02x", $0) }.joined()
}

private func report(_ kind: Int32, _ data: Data) {
    guard let cb = tokenCallback else { return }
    hex(data).withCString { cb(kind, $0) }
}

/// Registers the token callback and starts observing push-to-start tokens.
@_cdecl("jcode_live_set_token_callback")
public func jcode_live_set_token_callback(_ cb: TokenCallback?) {
    tokenCallback = cb
    #if canImport(ActivityKit)
    if #available(iOS 16.2, *) {
        LiveController.shared.observeStartTokens()
    }
    #endif
}

/// Applies the account-wide agents state (JSON `AgentsState`). An empty
/// state ends the activity. Returns 1 if an activity is showing afterwards.
@_cdecl("jcode_live_sync")
public func jcode_live_sync(_ json: UnsafePointer<CChar>?) -> Int32 {
    #if canImport(ActivityKit)
    if #available(iOS 16.2, *) {
        guard let data = str(json).data(using: .utf8),
              let state = try? JSONDecoder().decode(JcodeAgentsAttributes.ContentState.self, from: data)
        else { return 0 }
        return LiveController.shared.sync(state) ? 1 : 0
    }
    #endif
    return 0
}

@_cdecl("jcode_live_end")
public func jcode_live_end() {
    #if canImport(ActivityKit)
    if #available(iOS 16.2, *) {
        LiveController.shared.endAll()
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
/// Exactly one activity for all agents. The Worker updates it over APNs while
/// the app is suspended, so its token is reported whenever it changes.
@available(iOS 16.2, *)
final class LiveController {
    static let shared = LiveController()
    private var current: Activity<JcodeAgentsAttributes>?
    private var tokenTask: Task<Void, Never>?
    private var startTokenTask: Task<Void, Never>?

    func observeStartTokens() {
        // Pick up an activity the Worker started (push-to-start) or one left
        // from a previous launch, and ensure there is only one.
        let all = Activity<JcodeAgentsAttributes>.activities
        if current == nil, let first = all.first {
            adopt(first)
        }
        for a in all where a.id != current?.id {
            Task { await a.end(nil, dismissalPolicy: .immediate) }
        }
        guard startTokenTask == nil else { return }
        if #available(iOS 17.2, *) {
            startTokenTask = Task {
                for await data in Activity<JcodeAgentsAttributes>.pushToStartTokenUpdates {
                    report(1, data)
                }
            }
        }
        Task {
            for await a in Activity<JcodeAgentsAttributes>.activityUpdates where a.id != current?.id {
                await MainActor.run { self.adopt(a) }
            }
        }
    }

    private func adopt(_ a: Activity<JcodeAgentsAttributes>) {
        if let old = current, old.id != a.id {
            Task { await old.end(nil, dismissalPolicy: .immediate) }
        }
        current = a
        tokenTask?.cancel()
        tokenTask = Task {
            for await data in a.pushTokenUpdates {
                report(0, data)
            }
        }
    }

    func sync(_ state: JcodeAgentsAttributes.ContentState) -> Bool {
        if state.total == 0 {
            endAll()
            return false
        }
        let stale = Date().addingTimeInterval(20 * 60)
        if let cur = current, cur.activityState == .active || cur.activityState == .stale {
            Task { await cur.update(ActivityContent(state: state, staleDate: stale)) }
            return true
        }
        guard ActivityAuthorizationInfo().areActivitiesEnabled else { return false }
        do {
            let a = try Activity.request(
                attributes: JcodeAgentsAttributes(title: "jcode"),
                content: ActivityContent(state: state, staleDate: stale),
                pushType: .token
            )
            adopt(a)
            return true
        } catch {
            NSLog("jcode live activity: \(error)")
            return false
        }
    }

    func endAll() {
        current = nil
        tokenTask?.cancel()
        tokenTask = nil
        for a in Activity<JcodeAgentsAttributes>.activities {
            Task { await a.end(nil, dismissalPolicy: .after(Date().addingTimeInterval(60))) }
        }
    }
}
#endif
