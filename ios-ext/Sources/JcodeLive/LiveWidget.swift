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
                        ForEach(ctx.state.items.prefix(3), id: \.id) { item in
                            Link(destination: link(item.id)!) { Row(item: item, compact: true) }
                        }
                    }
                }
            } compactLeading: {
                if let p = ctx.state.progress {
                    ProgressRing(value: p, color: stateColor(top))
                } else {
                    Image(systemName: JcodeActivityText.symbol(top))
                        .foregroundColor(stateColor(top))
                        .accessibilityLabel(JcodeActivityText.label(top))
                }
            } compactTrailing: {
                Text(ctx.state.waiting > 0 ? "\(ctx.state.waiting)?" : "\(ctx.state.total)")
                    .font(.caption2).monospacedDigit().foregroundColor(stateColor(top))
            } minimal: {
                if let p = ctx.state.progress {
                    ProgressRing(value: p, color: stateColor(top))
                } else {
                    Text("\(ctx.state.total)").font(.caption2).monospacedDigit().foregroundColor(stateColor(top))
                }
            }
            .widgetURL(link(ctx.state.items.first?.id))
        }
    }
}

/// Todo bubbles: filled = done, ringed = in progress, dim = pending.
/// Past `maxDots` it becomes a thin bar so the row never wraps.
@available(iOS 16.2, *)
struct TodoBubbles: View {
    let done: Int
    let active: Int
    let total: Int
    let color: Color
    var maxDots = 8

    var body: some View {
        HStack(spacing: 4) {
            if total <= maxDots {
                HStack(spacing: 3) {
                    ForEach(0..<total, id: \.self) { i in
                        if i < done {
                            Circle().fill(color).frame(width: 7, height: 7)
                        } else if i < done + active {
                            Circle().stroke(color, lineWidth: 1.5).frame(width: 7, height: 7)
                        } else {
                            Circle().fill(Color.white.opacity(0.18)).frame(width: 7, height: 7)
                        }
                    }
                }
            } else {
                GeometryReader { g in
                    ZStack(alignment: .leading) {
                        Capsule().fill(Color.white.opacity(0.18))
                        Capsule().fill(color).frame(width: g.size.width * CGFloat(done) / CGFloat(max(total, 1)))
                    }
                }
                .frame(width: 44, height: 5)
                Text("\(done)/\(total)").font(.caption2.monospacedDigit()).foregroundColor(.gray)
            }
        }
        .accessibilityElement()
        .accessibilityLabel("\(done) of \(total) todos done")
    }
}

/// Overall todo progress ring for the island.
@available(iOS 16.2, *)
struct ProgressRing: View {
    let value: Double
    let color: Color

    var body: some View {
        ZStack {
            Circle().stroke(Color.white.opacity(0.2), lineWidth: 2.5)
            Circle().trim(from: 0, to: value).stroke(color, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: 16, height: 16)
        .accessibilityLabel("\(Int(value * 100)) percent of todos done")
    }
}

/// One root agent: origin icon, plan name, time, todo bubbles.
@available(iOS 16.2, *)
struct Row: View {
    let item: JcodeAgentsAttributes.Item
    let compact: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: item.originSymbol)
                .font(.caption)
                .foregroundColor(stateColor(item.state))
                .frame(width: 18)
                .accessibilityLabel("\(item.origin ?? "") \(JcodeActivityText.label(item.state))")
            VStack(alignment: .leading, spacing: 1) {
                Text(item.name).font(compact ? .caption : .subheadline).foregroundColor(.white).lineLimit(1)
                if !compact, item.state != "running" {
                    Text(JcodeActivityText.label(item.state)).font(.caption2).foregroundColor(stateColor(item.state)).lineLimit(1)
                }
            }
            .layoutPriority(1)
            Spacer(minLength: 4)
            if let start = item.sinceDate {
                // Ticks on the Lock Screen without any push.
                Text(timerInterval: start...Date.distantFuture, countsDown: false)
                    .font(.caption2.monospacedDigit()).foregroundColor(.gray)
                    .multilineTextAlignment(.trailing)
                    .frame(maxWidth: 44, alignment: .trailing)
            }
            if let total = item.total, total > 0 {
                TodoBubbles(done: item.done ?? 0, active: item.active ?? 0, total: total,
                            color: stateColor(item.state), maxDots: compact ? 6 : 8)
            }
        }
    }
}

@available(iOS 16.2, *)
struct LockScreenView: View {
    let state: JcodeAgentsAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(JcodeActivityText.headline(state)).font(.caption.bold())
                    .foregroundColor(stateColor(JcodeActivityText.top(state))).lineLimit(1)
                Spacer()
                if let t = state.todosTotal, t > 0 {
                    Text("\(state.todosDone ?? 0) of \(t) todos").font(.caption2.monospacedDigit()).foregroundColor(.gray)
                }
            }
            ForEach(state.items, id: \.id) { item in
                Link(destination: link(item.id)!) { Row(item: item, compact: false) }
            }
            let hidden = state.more ?? max(0, state.total - state.items.count)
            if hidden > 0 {
                Text("+\(hidden) more").font(.caption2).foregroundColor(.gray)
            }
        }
        .padding(14)
    }
}
