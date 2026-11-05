// Widget extension: Lock Screen Live Activity and Dynamic Island for a
// running jcode session. Tapping opens jcode://session/<id>.
import ActivityKit
import JcodeLiveShared
import SwiftUI
import WidgetKit

@main
struct JcodeWidgets: WidgetBundle {
    var body: some Widget {
        if #available(iOS 16.2, *) {
            JcodeLiveActivity()
        }
    }
}

private let accent = Color(red: 0.55, green: 0.70, blue: 1.0)

@available(iOS 16.2, *)
struct JcodeLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: JcodeActivityAttributes.self) { ctx in
            LockScreenView(attributes: ctx.attributes, state: ctx.state)
                .activityBackgroundTint(Color.black.opacity(0.85))
                .activitySystemActionForegroundColor(.white)
                .widgetURL(link(ctx.attributes))
        } dynamicIsland: { ctx in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Image(systemName: JcodeActivityText.symbol(ctx.state.activity))
                        .foregroundColor(accent)
                        .accessibilityHidden(true)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Text(timerInterval: ctx.state.startedAt...Date.distantFuture, countsDown: false)
                        .monospacedDigit()
                        .font(.caption)
                        .frame(maxWidth: 60)
                        .foregroundColor(.secondary)
                }
                DynamicIslandExpandedRegion(.center) {
                    Text(ctx.attributes.title).font(.headline).lineLimit(1)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    Text(statusLine(ctx.state)).font(.subheadline).foregroundColor(.secondary).lineLimit(1)
                }
            } compactLeading: {
                Image(systemName: JcodeActivityText.symbol(ctx.state.activity))
                    .foregroundColor(accent)
                    .accessibilityLabel(JcodeActivityText.label(ctx.state.activity))
            } compactTrailing: {
                Text(JcodeActivityText.label(ctx.state.activity)).font(.caption2).lineLimit(1).frame(maxWidth: 64)
            } minimal: {
                Image(systemName: JcodeActivityText.symbol(ctx.state.activity))
                    .foregroundColor(accent)
                    .accessibilityLabel(JcodeActivityText.label(ctx.state.activity))
            }
            .widgetURL(link(ctx.attributes))
        }
    }
}

@available(iOS 16.2, *)
private func link(_ a: JcodeActivityAttributes) -> URL? {
    URL(string: "jcode://session/\(a.sessionId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? "")")
}

@available(iOS 16.2, *)
private func statusLine(_ s: JcodeActivityAttributes.ContentState) -> String {
    let label = JcodeActivityText.label(s.activity)
    return s.detail.isEmpty ? label : "\(label): \(s.detail)"
}

@available(iOS 16.2, *)
struct LockScreenView: View {
    let attributes: JcodeActivityAttributes
    let state: JcodeActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: JcodeActivityText.symbol(state.activity))
                .font(.title2)
                .foregroundColor(accent)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(attributes.title).font(.headline).foregroundColor(.white).lineLimit(1)
                Text(statusLine(state)).font(.subheadline).foregroundColor(.gray).lineLimit(1)
            }
            Spacer()
            Text(timerInterval: state.startedAt...Date.distantFuture, countsDown: false)
                .monospacedDigit()
                .font(.caption)
                .foregroundColor(.gray)
                .frame(maxWidth: 60)
        }
        .padding(16)
    }
}
