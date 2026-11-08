// Widget extension: one Lock Screen Live Activity and Dynamic Island for all
// of the account's agents. Rows link to jcode://session/<id>.
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
private let attention = Color(red: 1.0, green: 0.75, blue: 0.3)
private let failure = Color(red: 1.0, green: 0.45, blue: 0.45)

private func stateColor(_ state: String) -> Color {
    switch state {
    case "waiting": return attention
    case "error": return failure
    default: return accent
    }
}

private func link(_ id: String?) -> URL? {
    guard let id, !id.isEmpty else { return URL(string: "jcode://") }
    return URL(string: "jcode://session/\(id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? "")")
}

@available(iOS 16.2, *)
struct JcodeLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: JcodeAgentsAttributes.self) { ctx in
            LockScreenView(state: ctx.state)
                .activityBackgroundTint(Color.black.opacity(0.85))
                .activitySystemActionForegroundColor(.white)
                .widgetURL(link(ctx.state.items.first?.id))
        } dynamicIsland: { ctx in
            let top = JcodeActivityText.top(ctx.state)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Image(systemName: JcodeActivityText.symbol(top))
                        .foregroundColor(stateColor(top))
                        .accessibilityHidden(true)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Text("\(ctx.state.total)").font(.headline).monospacedDigit().foregroundColor(.secondary)
                }
                DynamicIslandExpandedRegion(.center) {
                    Text(JcodeActivityText.headline(ctx.state)).font(.headline).lineLimit(1)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(ctx.state.items.prefix(2), id: \.id) { item in
                            Link(destination: link(item.id)!) { Row(item: item, compact: true) }
                        }
                    }
                }
            } compactLeading: {
                Image(systemName: JcodeActivityText.symbol(top))
                    .foregroundColor(stateColor(top))
                    .accessibilityLabel(JcodeActivityText.label(top))
            } compactTrailing: {
                Text(ctx.state.waiting > 0 ? "\(ctx.state.waiting)?" : "\(ctx.state.total)")
                    .font(.caption2).monospacedDigit().foregroundColor(stateColor(top))
            } minimal: {
                Text("\(ctx.state.total)").font(.caption2).monospacedDigit().foregroundColor(stateColor(top))
            }
            .widgetURL(link(ctx.state.items.first?.id))
        }
    }
}

@available(iOS 16.2, *)
struct Row: View {
    let item: JcodeAgentsAttributes.Item
    let compact: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: JcodeActivityText.symbol(item.state))
                .font(.caption)
                .foregroundColor(stateColor(item.state))
                .frame(width: 16)
                .accessibilityLabel(JcodeActivityText.label(item.state))
            Text(item.title).font(compact ? .caption : .subheadline).foregroundColor(.white).lineLimit(1)
            Spacer(minLength: 4)
            Text(item.detail.isEmpty ? JcodeActivityText.label(item.state) : item.detail)
                .font(.caption).foregroundColor(.gray).lineLimit(1)
        }
    }
}

@available(iOS 16.2, *)
struct LockScreenView: View {
    let state: JcodeAgentsAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("jcode").font(.caption).foregroundColor(.gray)
                Spacer()
                Text(JcodeActivityText.headline(state)).font(.caption.bold()).foregroundColor(stateColor(JcodeActivityText.top(state)))
            }
            ForEach(state.items, id: \.id) { item in
                Link(destination: link(item.id)!) { Row(item: item, compact: false) }
            }
            let hidden = state.total - state.items.count
            if hidden > 0 {
                Text("+\(hidden) more").font(.caption2).foregroundColor(.gray)
            }
        }
        .padding(14)
    }
}
