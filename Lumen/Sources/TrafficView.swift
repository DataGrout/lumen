import SwiftUI

private enum TrafficViewMode: String, CaseIterable {
    case live, aggregate

    var label: String {
        switch self {
        case .live: "Live"
        case .aggregate: "Aggregate"
        }
    }
}

private enum ProviderFilter: Hashable {
    case all
    case named(String)
    case other
}

struct TrafficView: View {
    let apiClient: APIClient
    @State private var viewMode = TrafficViewMode.live
    @State private var searchText = ""
    @State private var monitoredOnly = false
    @State private var providerFilter = ProviderFilter.all
    @State private var expandedHost: String? = nil
    @State private var cachedFiltered: [TrafficEntry] = []
    @State private var cachedAggregates: [HostAggregate] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            viewModeToggle
            filterBar
            // Wrap in a frame so empty and populated states take identical space,
            // eliminating the layout jump when traffic first appears.
            Group {
                switch viewMode {
                case .live:
                    liveView
                case .aggregate:
                    aggregateView
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .onAppear {
            apiClient.trafficTabActive = true
            refreshCaches()
        }
        .onDisappear { apiClient.trafficTabActive = false }
        .onChange(of: apiClient.trafficEntries.count) { refreshCaches() }
        .onChange(of: searchText) { refreshCaches() }
        .onChange(of: monitoredOnly) { refreshCaches() }
        .onChange(of: providerFilter) { refreshCaches() }
    }

    private func refreshCaches() {
        cachedFiltered = apiClient.trafficEntries.filter { entry in
            if monitoredOnly && !entry.isMonitored { return false }
            if !searchText.isEmpty && !entry.host.localizedCaseInsensitiveContains(searchText) {
                return false
            }
            if !matchesProvider(entry.host) { return false }
            return true
        }
        cachedAggregates = apiClient.trafficHostAggregates.filter { agg in
            if !searchText.isEmpty && !agg.host.localizedCaseInsensitiveContains(searchText) {
                return false
            }
            if !matchesProvider(agg.host) { return false }
            return true
        }
    }

    private var viewModeToggle: some View {
        HStack(spacing: 2) {
            ForEach(TrafficViewMode.allCases, id: \.self) { mode in
                modeButton(mode)
            }
        }
        .padding(2)
        .background(Color.white.opacity(0.04))
        .clipShape(RoundedRectangle(cornerRadius: 5))
    }

    private func modeButton(_ mode: TrafficViewMode) -> some View {
        Button(action: { viewMode = mode }) {
            Text(mode.label)
                .font(.system(size: 9, weight: .medium))
                .textCase(.uppercase)
                .tracking(0.3)
                .foregroundStyle(viewMode == mode ? .white.opacity(0.9) : .white.opacity(0.4))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 4)
                .background(viewMode == mode ? Color.white.opacity(0.08) : .clear)
                .clipShape(RoundedRectangle(cornerRadius: 4))
        }
        .buttonStyle(.plain)
    }

    private var filterBar: some View {
        VStack(spacing: 5) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.3))
                TextField("Filter by host...", text: $searchText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.8))

                Toggle(isOn: $monitoredOnly) {
                    Image(systemName: "eye")
                        .font(.system(size: 9))
                }
                .toggleStyle(.button)
                .font(.system(size: 9))
                .help("Show monitored only")
            }
            .padding(.horizontal, 7)
            .padding(.vertical, 4)
            .background(Color.white.opacity(0.04))
            .clipShape(RoundedRectangle(cornerRadius: 5))

            HStack(spacing: 3) {
                providerChip("All", filter: .all)
                providerChip("Cursor", filter: .named("cursor"))
                providerChip("OpenAI", filter: .named("openai"))
                providerChip("Anthropic", filter: .named("anthropic"))
                providerChip("Google", filter: .named("googleapis"))
                providerChip("Other", filter: .other)
            }
        }
    }

    private func providerChip(_ label: String, filter: ProviderFilter) -> some View {
        let isActive = providerFilter == filter
        return Button(action: { providerFilter = filter }) {
            Text(label)
                .font(.system(size: 8, weight: .medium))
                .foregroundStyle(isActive ? .white.opacity(0.9) : .white.opacity(0.4))
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(isActive ? Color.white.opacity(0.1) : .clear)
                .clipShape(RoundedRectangle(cornerRadius: 3))
        }
        .buttonStyle(.plain)
    }

    private func matchesProvider(_ host: String) -> Bool {
        switch providerFilter {
        case .all:
            return true
        case .named(let keyword):
            return host.contains(keyword)
        case .other:
            let known = ["openai", "anthropic", "googleapis", "cursor"]
            return !known.contains(where: { host.contains($0) })
        }
    }

    // MARK: - Live View

    private var liveView: some View {
        Group {
            if cachedFiltered.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "antenna.radiowaves.left.and.right")
                        .font(.system(size: 28))
                        .foregroundStyle(.white.opacity(0.08))
                    Text("No traffic recorded yet")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.25))
                    Text("Requests through the proxy will appear here")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.15))
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(cachedFiltered) { entry in
                            TrafficRow(entry: entry)
                        }
                    }
                }
            }
        }
    }

    // MARK: - Aggregate View

    private var maxRequests: Int {
        cachedAggregates.map(\.totalRequests).max() ?? 1
    }

    private var aggregateView: some View {
        Group {
            if cachedAggregates.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "chart.bar.xaxis")
                        .font(.system(size: 28))
                        .foregroundStyle(.white.opacity(0.08))
                    Text("No host data yet")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.25))
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 3) {
                        ForEach(cachedAggregates) { agg in
                            TrafficHostRow(
                                aggregate: agg,
                                maxRequests: maxRequests,
                                isExpanded: expandedHost == agg.host,
                                onTap: {
                                    expandedHost = expandedHost == agg.host ? nil : agg.host
                                }
                            )
                        }
                    }
                }
            }
        }
    }
}

// MARK: - Traffic Row (Live view)

struct TrafficRow: View {
    let entry: TrafficEntry

    private var statusColor: Color {
        switch entry.status {
        case 200..<300: return .green
        case 400..<500: return .yellow
        case 500..<600: return .red
        default: return .white.opacity(0.4)
        }
    }

    var body: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(entry.isMonitored ? .green : .white.opacity(0.15))
                .frame(width: 6, height: 6)

            Text(entry.method)
                .font(.system(size: 8, weight: .bold, design: .monospaced))
                .foregroundStyle(.white.opacity(0.5))
                .frame(width: 30, alignment: .leading)

            VStack(alignment: .leading, spacing: 0) {
                Text(entry.host)
                    .font(.system(size: 9, weight: .medium))
                    .foregroundStyle(.white.opacity(0.7))
                    .lineLimit(1)
                Text(entry.path)
                    .font(.system(size: 8, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.3))
                    .lineLimit(1)
            }

            Spacer()

            VStack(alignment: .trailing, spacing: 0) {
                Text("\(entry.status)")
                    .font(.system(size: 9, weight: .semibold, design: .monospaced))
                    .foregroundStyle(statusColor)

                if entry.isMonitored && !entry.dataCaptured.isEmpty {
                    Text(entry.dataCaptured.joined(separator: ", "))
                        .font(.system(size: 7))
                        .foregroundStyle(.green.opacity(0.5))
                } else if !entry.isMonitored {
                    Text("passthrough")
                        .font(.system(size: 7))
                        .foregroundStyle(.white.opacity(0.2))
                }
            }

            Text(formatBytes(entry.responseBytes))
                .font(.system(size: 8, design: .monospaced))
                .foregroundStyle(.white.opacity(0.35))
                .frame(width: 36, alignment: .trailing)

            Text("\(entry.latencyMs)ms")
                .font(.system(size: 8, design: .monospaced))
                .foregroundStyle(.white.opacity(0.3))
                .frame(width: 40, alignment: .trailing)
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 4)
        .background(Color.white.opacity(entry.isMonitored ? 0.04 : 0.02))
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }
}

// MARK: - Aggregate Host Row

struct TrafficHostRow: View {
    let aggregate: HostAggregate
    let maxRequests: Int
    let isExpanded: Bool
    let onTap: () -> Void

    private var barWidth: CGFloat {
        guard maxRequests > 0 else { return 0 }
        return CGFloat(aggregate.totalRequests) / CGFloat(maxRequests)
    }

    private var hostColor: Color {
        if aggregate.host.contains("openai") { return .green }
        if aggregate.host.contains("anthropic") { return .yellow }
        if aggregate.host.contains("google") { return .blue }
        return .purple
    }

    var body: some View {
        VStack(spacing: 0) {
            Button(action: onTap) {
                VStack(spacing: 4) {
                    HStack(spacing: 6) {
                        Circle()
                            .fill(hostColor)
                            .frame(width: 6, height: 6)
                        Text(aggregate.host)
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.white.opacity(0.7))
                            .lineLimit(1)
                        Spacer()
                        Text("\(aggregate.totalRequests) req")
                            .font(.system(size: 9, design: .monospaced))
                            .foregroundStyle(.white.opacity(0.5))
                        Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                            .font(.system(size: 7))
                            .foregroundStyle(.white.opacity(0.2))
                    }

                    GeometryReader { geo in
                        RoundedRectangle(cornerRadius: 2)
                            .fill(hostColor.opacity(0.3))
                            .frame(width: geo.size.width * barWidth, height: 3)
                    }
                    .frame(height: 3)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(Color.white.opacity(0.03))
                .clipShape(RoundedRectangle(cornerRadius: 5))
            }
            .buttonStyle(.plain)

            if isExpanded {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text("↑ \(formatBytes(aggregate.totalRequestBytes))")
                        Text("↓ \(formatBytes(aggregate.totalResponseBytes))")
                        Text("⚡ \(aggregate.requestsMonitored) monitored")
                        Text("⏱ \(String(format: "%.0f", aggregate.avgLatencyMs))ms avg")
                    }
                    .font(.system(size: 8, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.35))
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 4)
            }
        }
    }

}

func formatBytes(_ bytes: Int) -> String {
    if bytes >= 1_048_576 { return String(format: "%.1fMB", Double(bytes) / 1_048_576) }
    if bytes >= 1_024 { return String(format: "%.1fKB", Double(bytes) / 1_024) }
    return "\(bytes)B"
}
