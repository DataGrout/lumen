import SwiftUI

enum FeedItem: Identifiable {
    case event(UsageEvent)
    case lapMarker(LapSnapshot)

    var id: String {
        switch self {
        case .event(let e): return "event-\(e.id)"
        case .lapMarker(let l): return "lap-\(l.lapNumber)"
        }
    }

    var sortDate: Date {
        switch self {
        case .event(let e): return parseISO(e.timestamp)
        case .lapMarker(let l): return parseISO(l.endedAt)
        }
    }
}

private let isoFormatter: ISO8601DateFormatter = {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return f
}()

private let isoFallback: ISO8601DateFormatter = {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime]
    return f
}()

private func parseISO(_ str: String) -> Date {
    isoFormatter.date(from: str) ?? isoFallback.date(from: str) ?? Date.distantPast
}

struct EventFeedView: View {
    let events: [UsageEvent]
    var laps: [LapSnapshot] = []

    private var feedItems: [FeedItem] {
        var items: [FeedItem] = events.map { .event($0) }
        for lap in laps {
            items.append(.lapMarker(lap))
        }
        return items.sorted { $0.sortDate > $1.sortDate }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("Recent Activity")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.white.opacity(0.6))
                    .textCase(.uppercase)
                    .tracking(0.5)
                Spacer()
                Text("\(events.count) events")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.4))
            }

            if events.isEmpty {
                VStack(spacing: 4) {
                    Text("Waiting for LLM API calls...")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.25))
                    Text("Configure your app to use the proxy")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.15))
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 20)
            } else {
                ScrollView {
                    LazyVStack(spacing: 3) {
                        ForEach(feedItems) { item in
                            switch item {
                            case .event(let event):
                                EventRow(event: event)
                            case .lapMarker(let lap):
                                LapDivider(lap: lap)
                            }
                        }
                    }
                }
                .frame(maxHeight: 180)
            }
        }
    }
}

struct LapDivider: View {
    let lap: LapSnapshot

    var body: some View {
        HStack(spacing: 6) {
            Rectangle()
                .fill(Color.orange.opacity(0.35))
                .frame(height: 1)
            HStack(spacing: 5) {
                Image(systemName: "flag.fill")
                    .font(.system(size: 8))
                    .foregroundStyle(.orange.opacity(0.8))
                Text("Lap \(lap.lapNumber)")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.orange.opacity(0.8))
                Text("·")
                    .foregroundStyle(.white.opacity(0.25))
                Text(formatTokens(lap.totalTokens) + " tok")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.5))
                Text("·")
                    .foregroundStyle(.white.opacity(0.25))
                Text(formatCost(lap.cost))
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.orange.opacity(0.65))
            }
            .fixedSize()
            Rectangle()
                .fill(Color.orange.opacity(0.35))
                .frame(height: 1)
        }
        .padding(.vertical, 5)
    }
}

struct EventRow: View {
    let event: UsageEvent

    private var freshIn: Int { event.usage.inputTokens }
    private var out: Int { event.usage.outputTokens }
    private var cacheRead: Int { event.usage.cacheReadTokens ?? 0 }
    private var cacheWrite: Int { event.usage.cacheCreationTokens ?? 0 }
    // totalIn = all tokens you sent (fresh + newly cached)
    private var totalIn: Int { freshIn + cacheWrite }

    @ViewBuilder
    private var tokenDisplay: some View {
        if cacheRead > 0 || cacheWrite > 0 {
            // Show: totalIn · out · ~cacheRead
            HStack(spacing: 3) {
                if totalIn > 0 {
                    Text(formatTokens(totalIn))
                        .foregroundStyle(.white.opacity(0.5))
                    Text("·")
                        .foregroundStyle(.white.opacity(0.12))
                }
                Text(formatTokens(out))
                    .foregroundStyle(.white.opacity(0.65))
                if cacheRead > 0 {
                    Text("·")
                        .foregroundStyle(.white.opacity(0.12))
                    Text("~\(formatTokens(cacheRead))")
                        .foregroundStyle(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.75))
                }
            }
            .font(.system(size: 10))
            .monospacedDigit()
        } else {
            Text(formatTokens(freshIn + out))
                .font(.system(size: 10))
                .foregroundStyle(.white.opacity(0.55))
                .monospacedDigit()
        }
    }

    private var eventDetailCard: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack {
                Text(event.model)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.85))
                Spacer()
                Text(formatTimestamp(event.timestamp))
                    .font(.system(size: 9, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.35))
            }

            Divider().background(Color.white.opacity(0.1))

            VStack(spacing: 3) {
                detailRow("New input",   formatTokens(freshIn),    .blue.opacity(0.7))
                if cacheRead > 0 {
                    detailRow("Cache read", formatTokens(cacheRead),  .teal.opacity(0.8))
                }
                if cacheWrite > 0 {
                    detailRow("Cache write", formatTokens(cacheWrite), Color(red: 0.90, green: 0.55, blue: 0.15).opacity(0.8))
                }
                detailRow("Output",     formatTokens(out),         .orange.opacity(0.75))
            }

            Divider().background(Color.white.opacity(0.1))

            HStack {
                detailRow("Cost",    formatCost(event.cost.totalCost),     .green)
                Spacer()
                if event.cost.cacheReadSavings > 0.0001 {
                    Text("saved \(formatCost(event.cost.cacheReadSavings))")
                        .font(.system(size: 9))
                        .foregroundStyle(.white.opacity(0.3))
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: NSColor(red: 0.06, green: 0.06, blue: 0.10, alpha: 1.0)))
        .clipShape(RoundedRectangle(cornerRadius: 7))
        .overlay(RoundedRectangle(cornerRadius: 7).stroke(Color.white.opacity(0.1), lineWidth: 1))
    }

    private func detailRow(_ label: String, _ value: String, _ color: Color) -> some View {
        HStack {
            Text(label)
                .font(.system(size: 9))
                .foregroundStyle(.white.opacity(0.4))
                .frame(width: 72, alignment: .leading)
            Text(value)
                .font(.system(size: 9, weight: .semibold, design: .monospaced))
                .foregroundStyle(color)
        }
    }

    private func formatTimestamp(_ iso: String) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = f.date(from: iso) ?? ISO8601DateFormatter().date(from: iso) else { return iso }
        let out = DateFormatter()
        out.dateFormat = "HH:mm:ss"
        return out.string(from: date)
    }

    private var providerColor: Color {
        switch event.provider {
        case "openai": return .green
        case "anthropic": return .yellow
        case "google": return .blue
        default: return .purple
        }
    }

    @State private var showDetail = false

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Text(event.provider)
                    .font(.system(size: 8, weight: .semibold))
                    .textCase(.uppercase)
                    .tracking(0.3)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(providerColor)
                    .foregroundStyle(.black.opacity(0.8))
                    .clipShape(RoundedRectangle(cornerRadius: 3))

                Text(event.model)
                    .font(.system(size: 11))
                    .foregroundStyle(.white.opacity(0.75))
                    .lineLimit(1)

                Spacer()

                tokenDisplay

                Text(formatCost(event.cost.totalCost))
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.green)
                    .monospacedDigit()
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)

            if showDetail {
                eventDetailCard
                    .padding(.top, 3)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .background(showDetail ? Color.white.opacity(0.06) : Color.white.opacity(0.03))
        .clipShape(RoundedRectangle(cornerRadius: 5))
        .contentShape(Rectangle())
        .onTapGesture { withAnimation(.easeInOut(duration: 0.15)) { showDetail.toggle() } }
    }
}

func formatTokens(_ n: Int) -> String {
    if n >= 1_000_000 { return String(format: "%.1fM", Double(n) / 1_000_000) }
    if n >= 1_000 { return String(format: "%.1fk", Double(n) / 1_000) }
    return "\(n)"
}

func formatCost(_ c: Double) -> String {
    if c < 0.001 { return "$0.00" }
    if c < 0.01 { return String(format: "$%.4f", c) }
    if c < 1 { return String(format: "$%.3f", c) }
    return String(format: "$%.2f", c)
}
