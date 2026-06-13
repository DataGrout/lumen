import SwiftUI
import AppKit
import UniformTypeIdentifiers

enum AppTab: String, CaseIterable {
    case monitor, hosts, traffic, settings, launch

    var label: String {
        switch self {
        case .monitor: "Monitor"
        case .hosts: "Endpoints"
        case .traffic: "Traffic"
        case .settings: "Settings"
        case .launch: "Launch"
        }
    }
}

struct PopoverView: View {
    let apiClient: APIClient
    let daemonManager: DaemonManager
    @State private var activeTab = AppTab.monitor
    @State private var showLapHistory = false
    @State private var selectedLapIds: Set<Int> = []
    @State private var showCompare = false
    @State private var namingLap = false
    @State private var lapNameInput = ""
    @FocusState private var lapNameFocused: Bool
    @AppStorage("lumen.dgCtaDismissedAt") private var dgCtaDismissedAt: Double = 0

    private var dgCtaDismissed: Bool {
        Date().timeIntervalSince1970 - dgCtaDismissedAt < 7 * 24 * 3600
    }

    var body: some View {
        VStack(spacing: 0) {
            // Top content: header, tabs, scrolling tab body
            VStack(spacing: 14) {
                header
                tabBar
                tabContent
            }
            .padding(.horizontal, 14)
            .padding(.top, 14)
            .padding(.bottom, 10)

            // Persistent footer — globally-relevant actions (Dashboard / Quit)
            // that should be one click away from any tab, not just Monitor.
            // Lives outside the scrolling content so it stays visible.
            persistentFooter
        }
        .frame(width: 400, height: 640)
        .background(Color(nsColor: NSColor(red: 0.04, green: 0.04, blue: 0.06, alpha: 1)))
        .onReceive(NotificationCenter.default.publisher(for: .lumenShowTab)) { note in
            if let raw = note.object as? String, let tab = AppTab(rawValue: raw) {
                activeTab = tab
            }
        }
    }

    /// The Dashboard URL is hardcoded to the daemon's default API port. If a
    /// user runs lumen-core with `--api-port N` for N ≠ 9091, this would
    /// drift — but every other client-side reference (APIClient.baseURL,
    /// build_dmg.sh, README) is also pinned to 9091, so consolidating later
    /// is a single refactor, not piecemeal threading of the port through.
    private static let dashboardURL = URL(string: "http://127.0.0.1:9091/dashboard")!

    private var persistentFooter: some View {
        VStack(spacing: 6) {
            // Lap-naming text field appears when the user clicks Lap; lives
            // here (not in monitorView) so committing it doesn't push the
            // footer off-screen.
            if namingLap {
                lapNamingRow
            }

            // Context-specific row: Lap + Clear, Monitor tab only. Stacked
            // above the global Dashboard/Quit row so the buttons closest to
            // your monitoring data are the ones that act on it.
            if activeTab == .monitor {
                HStack(spacing: 6) {
                    lapFooterButton
                    clearFooterButton
                }
            }

            // Global row: Dashboard + Quit, always visible regardless of tab.
            HStack(spacing: 6) {
                dashboardFooterButton
                quitFooterButton
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(Color.white.opacity(0.02))
        .overlay(
            Rectangle()
                .frame(height: 1)
                .foregroundStyle(Color.white.opacity(0.05)),
            alignment: .top
        )
    }

    // MARK: - Footer button components
    // Broken out so the persistentFooter VStack stays scannable. Each one
    // matches the original .plain-button styling pattern used elsewhere in
    // the popover (color-tinted background + matching stroke + contentShape
    // so the full padded area is hit-testable).

    private var lapFooterButton: some View {
        Button(action: {
            guard !namingLap else { return }
            lapNameInput = ""
            namingLap = true
            lapNameFocused = true
        }) {
            HStack(spacing: 4) {
                Image(systemName: "stopwatch").font(.system(size: 9))
                Text("Lap")
                    .font(.system(size: 10, weight: .medium))
                    .textCase(.uppercase)
                    .tracking(0.4)
            }
            .foregroundStyle(.orange.opacity(0.85))
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(Color.orange.opacity(0.10))
            .clipShape(RoundedRectangle(cornerRadius: 7))
            .overlay(RoundedRectangle(cornerRadius: 7)
                .stroke(Color.orange.opacity(0.35), lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
    }

    private var clearFooterButton: some View {
        Button(action: { Task { await apiClient.clearSession() } }) {
            HStack(spacing: 4) {
                Image(systemName: "trash").font(.system(size: 9))
                Text("Clear")
                    .font(.system(size: 10, weight: .medium))
                    .textCase(.uppercase)
                    .tracking(0.4)
            }
            .foregroundStyle(.white.opacity(0.6))
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(Color.white.opacity(0.04))
            .clipShape(RoundedRectangle(cornerRadius: 7))
            .overlay(RoundedRectangle(cornerRadius: 7)
                .stroke(Color.white.opacity(0.1), lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
    }

    private var dashboardFooterButton: some View {
        Button(action: { NSWorkspace.shared.open(Self.dashboardURL) }) {
            HStack(spacing: 4) {
                Image(systemName: "safari").font(.system(size: 9))
                Text("Dashboard")
                    .font(.system(size: 10, weight: .medium))
                    .textCase(.uppercase)
                    .tracking(0.4)
            }
            .foregroundStyle(Color(red: 0.4, green: 0.7, blue: 0.95).opacity(0.85))
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(Color(red: 0.4, green: 0.7, blue: 0.95).opacity(0.10))
            .clipShape(RoundedRectangle(cornerRadius: 7))
            .overlay(RoundedRectangle(cornerRadius: 7)
                .stroke(Color(red: 0.4, green: 0.7, blue: 0.95).opacity(0.30), lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
    }

    private var quitFooterButton: some View {
        Button(action: { NSApp.terminate(nil) }) {
            HStack(spacing: 4) {
                Image(systemName: "power").font(.system(size: 9))
                Text("Quit")
                    .font(.system(size: 10, weight: .medium))
                    .textCase(.uppercase)
                    .tracking(0.4)
            }
            .foregroundStyle(.red.opacity(0.7))
            .frame(maxWidth: .infinity)
            .padding(.vertical, 8)
            .background(Color.red.opacity(0.07))
            .clipShape(RoundedRectangle(cornerRadius: 7))
            .overlay(RoundedRectangle(cornerRadius: 7)
                .stroke(Color.red.opacity(0.20), lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
    }

    private var header: some View {
        HStack {
            HStack(spacing: 6) {
                Image(systemName: "circle.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(.orange)
                    .shadow(color: .orange.opacity(0.7), radius: 6)
                    .shadow(color: .orange.opacity(0.3), radius: 12)
                VStack(alignment: .leading, spacing: 0) {
                    Text("Lumen")
                        .font(.system(size: 15, weight: .bold))
                        .foregroundStyle(.white)
                    Text("by DataGrout")
                        .font(.system(size: 8, weight: .medium))
                        .foregroundStyle(.white.opacity(0.45))
                        .tracking(0.2)
                }
            }
            Spacer()
            statusIndicator
        }
    }

    private var statusIndicator: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(apiClient.connected ? .green : .white.opacity(0.15))
                .frame(width: 7, height: 7)
                .shadow(color: apiClient.connected ? .green.opacity(0.5) : .clear, radius: 4)
            VStack(alignment: .leading, spacing: 0) {
                Text(apiClient.connected ? "Connected" : "Disconnected")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.7))
                Text(":\(apiClient.proxyConfig.port)")
                    .font(.system(size: 8, design: .monospaced))
                    .foregroundStyle(.white.opacity(0.35))
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(Color.white.opacity(0.05))
        .clipShape(RoundedRectangle(cornerRadius: 7))
    }

    private var tabBar: some View {
        HStack(spacing: 2) {
            ForEach(AppTab.allCases, id: \.self) { tab in
                tabButton(tab)
            }
        }
        .padding(3)
        .background(Color.white.opacity(0.04))
        .clipShape(RoundedRectangle(cornerRadius: 7))
    }

    private func tabButton(_ tab: AppTab) -> some View {
        Button(action: { activeTab = tab }) {
            Text(tab.label)
                .font(.system(size: 10, weight: .medium))
                .textCase(.uppercase)
                .tracking(0.4)
                .foregroundStyle(activeTab == tab ? .white.opacity(0.95) : .white.opacity(0.5))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 5)
                .background(activeTab == tab ? Color.white.opacity(0.10) : Color.clear)
                .clipShape(RoundedRectangle(cornerRadius: 5))
                // Without an explicit content shape, SwiftUI's .plain button
                // style only treats the rendered text glyph as hit-testable —
                // clicking the padded background area registered as "miss" and
                // required pixel-precise aim. Rectangle() makes the full
                // frame (incl. background) clickable.
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .focusable(false)
    }

    @ViewBuilder
    private var tabContent: some View {
        switch activeTab {
        case .monitor:
            monitorView
        case .hosts:
            ScrollView { HostsView(apiClient: apiClient) }
        case .traffic:
            TrafficView(apiClient: apiClient)
        case .settings:
            ScrollView { SettingsView(apiClient: apiClient, daemonManager: daemonManager) }
        case .launch:
            LaunchersView(apiClient: apiClient, daemonManager: daemonManager)
        }
    }

    private var monitorView: some View {
        ScrollView {
            VStack(spacing: 14) {
                if !apiClient.connected || !apiClient.proxyConfig.running {
                    captureBanner
                }
                if apiClient.dgStatus?.isExpiredSession == true {
                    dgExpiredBanner
                }
                gaugeRow
                tokenBar
                summaryRow
                if !apiClient.laps.isEmpty {
                    lapSection
                }
                if !dgCtaDismissed, apiClient.dgStatus?.connected != true,
                   let cta = dgCTAReason() {
                    dgCTABanner(reason: cta)
                }
                EventFeedView(events: apiClient.recentEvents, laps: apiClient.laps)
                // Lap / Clear moved to persistentFooter — they're now always
                // visible at the bottom of the popover when on Monitor,
                // stacked above the global Dashboard / Quit row.
            }
        }
    }

    @ViewBuilder
    // Shown on Monitor when the DG identity cert has expired and sync has
    // fallen back to the sync-token bearer. "Reconnect" jumps to Settings →
    // DataGrout, where the actual OAuth reconnect lives.
    private var dgExpiredBanner: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.lock.fill")
                .font(.system(size: 14))
                .foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 1) {
                Text("DataGrout session expired")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.9))
                Text("Syncing on a fallback token — reconnect to restore secure mTLS")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.45))
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer()
            Button(action: { activeTab = .settings }) {
                Text("Reconnect")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.orange)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(Color.orange.opacity(0.12))
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                    .overlay(RoundedRectangle(cornerRadius: 5).stroke(Color.orange.opacity(0.35), lineWidth: 1))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .focusable(false)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color.orange.opacity(0.07))
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.orange.opacity(0.2), lineWidth: 1))
    }

    private var captureBanner: some View {
        let (icon, message, detail, color): (String, String, String, Color) = {
            if !apiClient.connected {
                return ("exclamationmark.triangle.fill",
                        "Daemon not running",
                        "Traffic is not being captured",
                        .red)
            } else {
                return ("pause.circle.fill",
                        "Proxy not running",
                        "LLM traffic is not being captured",
                        Color(red: 0.95, green: 0.6, blue: 0.1))
            }
        }()

        return HStack(spacing: 10) {
            Image(systemName: icon)
                .font(.system(size: 14))
                .foregroundStyle(color)

            VStack(alignment: .leading, spacing: 1) {
                Text(message)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.white.opacity(0.9))
                Text(detail)
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.45))
            }

            Spacer()

            if !apiClient.connected {
                Button(action: {
                    daemonManager.start()
                    // Poll immediately after restart attempt so UI responds quickly
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { apiClient.pollNow() }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { apiClient.pollNow() }
                }) {
                    Text("Restart")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(color)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(color.opacity(0.12))
                        .clipShape(RoundedRectangle(cornerRadius: 5))
                        .overlay(RoundedRectangle(cornerRadius: 5).stroke(color.opacity(0.35), lineWidth: 1))
                }
                .buttonStyle(.plain)
                .focusable(false)
            } else if !apiClient.proxyConfig.running {
                Button(action: { Task { await apiClient.startProxy() } }) {
                    Text("Start")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(color)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(color.opacity(0.12))
                        .clipShape(RoundedRectangle(cornerRadius: 5))
                        .overlay(RoundedRectangle(cornerRadius: 5).stroke(color.opacity(0.35), lineWidth: 1))
                }
                .buttonStyle(.plain)
                .focusable(false)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(color.opacity(0.07))
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(color.opacity(0.2), lineWidth: 1))
    }

    private var gaugeRow: some View {
        HStack(spacing: 0) {
            ArcGauge(
                value: apiClient.stats.sessionCost,
                max: Swift.max(apiClient.stats.sessionCost * 2, 1),
                label: "Lap Cost",
                prefix: "$",
                color: .orange,
                size: 110
            )
            ArcGauge(
                value: apiClient.stats.tokensPerMinute,
                max: Swift.max(apiClient.stats.tokensPerMinute * 1.5, 1000),
                label: "Rate",
                unit: "tok/min",
                color: .blue,
                size: 110
            )
            ArcGauge(
                value: apiClient.stats.totalCost,
                max: Swift.max(apiClient.stats.totalCost * 2, 0.1),
                label: "Total",
                prefix: "$",
                color: .green,
                size: 110
            )
        }
        .padding(.vertical, 4)
    }

    private var tokenBar: some View {
        let fresh = apiClient.stats.sessionInputTokens
        let cacheWrite = apiClient.stats.sessionCacheCreationTokens
        let cacheRead = apiClient.stats.sessionCacheReadTokens
        let output = apiClient.stats.sessionOutputTokens
        let totalIn = fresh + cacheWrite  // all tokens you sent this lap
        let total = max(totalIn + cacheRead + output, 1)

        return VStack(alignment: .leading, spacing: 4) {
            HStack {
                HStack(spacing: 4) {
                    Text("Lap \(apiClient.stats.currentLap)")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.orange.opacity(0.7))
                        .textCase(.uppercase)
                        .tracking(0.5)
                    Text("Tokens")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.5))
                        .textCase(.uppercase)
                        .tracking(0.5)
                }
                Spacer()
                Text(formatTokens(total))
                    .font(.system(size: 11))
                    .foregroundStyle(.white.opacity(0.8))
                    .monospacedDigit()
            }

            GeometryReader { geo in
                let w = geo.size.width
                HStack(spacing: 0) {
                    Rectangle().fill(Color.blue)
                        .frame(width: w * CGFloat(totalIn) / CGFloat(total))
                    Rectangle().fill(Color.teal)
                        .frame(width: w * CGFloat(cacheRead) / CGFloat(total))
                    Rectangle().fill(Color.orange)
                        .frame(width: w * CGFloat(output) / CGFloat(total))
                }
            }
            .frame(height: 6)
            .background(Color.white.opacity(0.06))
            .clipShape(RoundedRectangle(cornerRadius: 3))

            HStack(spacing: 10) {
                legendDot(.blue, "In: \(formatTokens(totalIn))")
                legendDot(.teal, "Cache: \(formatTokens(cacheRead))")
                legendDot(.orange, "Out: \(formatTokens(output))")
            }
        }
    }

    private func legendDot(_ color: Color, _ text: String) -> some View {
        HStack(spacing: 4) {
            Circle().fill(color).frame(width: 7, height: 7)
                .shadow(color: color.opacity(0.5), radius: 2)
            Text(text)
                .font(.system(size: 10))
                .foregroundStyle(.white.opacity(0.65))
                .monospacedDigit()
        }
    }

    private var summaryRow: some View {
        HStack(spacing: 6) {
            summaryCard("\(apiClient.stats.eventCount)", label: "Calls")
            summaryCard(formatTokens(apiClient.stats.totalTokens), label: "Total Tokens")
            summaryCard(apiClient.stats.topModel ?? "—", label: "Top Model")
        }
    }

    private func summaryCard(_ value: String, label: String) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.white.opacity(0.9))
                .monospacedDigit()
                .lineLimit(1)
                .minimumScaleFactor(0.6)
            Text(label)
                .font(.system(size: 9))
                .foregroundStyle(.white.opacity(0.45))
                .textCase(.uppercase)
                .tracking(0.4)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
        .background(Color.white.opacity(0.05))
        .clipShape(RoundedRectangle(cornerRadius: 7))
    }

    // MARK: - Lap History

    private var lapSection: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Button(action: { showLapHistory.toggle() }) {
                    HStack(spacing: 4) {
                        Text("Lap History")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.white.opacity(0.55))
                            .textCase(.uppercase)
                            .tracking(0.5)
                        Text("(\(apiClient.laps.count))")
                            .font(.system(size: 10))
                            .foregroundStyle(.white.opacity(0.3))
                    }
                }
                .buttonStyle(.plain)
                .focusable(false)

                Spacer()

                Menu {
                    Button("Export as JSON") { exportLaps(format: .json) }
                    Button("Export as CSV")  { exportLaps(format: .csv)  }
                    Divider()
                    if apiClient.dgStatus?.connected == true {
                        Button("Reshape & analyze in DataGrout") { exportLaps(format: .json) }
                    } else {
                        Button("Connect DataGrout for dashboards & analysis") {}
                            .disabled(true)
                    }
                } label: {
                    Image(systemName: "square.and.arrow.down")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.3))
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .focusable(false)

                Button(action: { showLapHistory.toggle() }) {
                    Image(systemName: showLapHistory ? "chevron.up" : "chevron.down")
                        .font(.system(size: 9))
                        .foregroundStyle(.white.opacity(0.3))
                }
                .buttonStyle(.plain)
                .focusable(false)
            }

            if showLapHistory {
                LazyVStack(spacing: 3) {
                    ForEach(apiClient.laps.reversed()) { lap in
                        LapRow(
                            lap: lap,
                            previousLap: previousLap(for: lap),
                            isSelected: selectedLapIds.contains(lap.lapNumber),
                            onToggle: {
                                if selectedLapIds.contains(lap.lapNumber) {
                                    selectedLapIds.remove(lap.lapNumber)
                                } else {
                                    selectedLapIds.insert(lap.lapNumber)
                                }
                                showCompare = false
                            }
                        )
                    }
                }

                lapSelectionFooter
            }
        }
    }

    @ViewBuilder
    private var lapSelectionFooter: some View {
        if selectedLapIds.count >= 2 {
            VStack(spacing: 6) {
                HStack(spacing: 6) {
                    Button(action: { showCompare.toggle() }) {
                        HStack(spacing: 4) {
                            Image(systemName: "chart.bar.xaxis")
                                .font(.system(size: 9))
                            Text(showCompare ? "Hide comparison" : "Compare \(selectedLapIds.count) laps")
                                .font(.system(size: 10, weight: .medium))
                        }
                        .foregroundStyle(.orange.opacity(0.85))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 6)
                        .background(Color.orange.opacity(0.08))
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                        .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.orange.opacity(0.25), lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                    .focusable(false)

                    Button(action: { selectedLapIds.removeAll(); showCompare = false }) {
                        Text("Clear")
                            .font(.system(size: 10))
                            .foregroundStyle(.white.opacity(0.35))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                            .background(Color.white.opacity(0.04))
                            .clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                    .buttonStyle(.plain)
                    .focusable(false)
                }
                .padding(.top, 3)

                if showCompare {
                    lapCompareView
                }
            }
        } else if selectedLapIds.count == 1 {
            Text("Tap another lap to compare")
                .font(.system(size: 9))
                .foregroundStyle(.white.opacity(0.25))
                .frame(maxWidth: .infinity)
                .padding(.top, 4)
        } else if apiClient.laps.count >= 2 {
            HStack(spacing: 4) {
                Image(systemName: "sparkles")
                    .font(.system(size: 8))
                Text(apiClient.dgStatus?.connected == true
                     ? "Select laps to compare · analyze deeper in DataGrout"
                     : "Select laps to compare · connect DataGrout for analysis")
                    .font(.system(size: 9))
            }
            .foregroundStyle(.white.opacity(0.22))
            .frame(maxWidth: .infinity)
            .padding(.top, 2)
        }
    }

    private var lapCompareView: some View {
        let selected = apiClient.laps
            .filter { selectedLapIds.contains($0.lapNumber) }
            .sorted { $0.lapNumber < $1.lapNumber }

        return VStack(spacing: 6) {
            HStack(alignment: .top, spacing: 6) {
                ForEach(selected) { lap in
                    lapCompareCard(lap)
                }
            }

            if apiClient.dgStatus?.connected == true {
                Button(action: { exportLaps(format: .json) }) {
                    HStack(spacing: 4) {
                        Image(systemName: "sparkles")
                            .font(.system(size: 9))
                        Text("Analyze in DataGrout")
                            .font(.system(size: 9, weight: .medium))
                    }
                    .foregroundStyle(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.8))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 5)
                    .background(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.07))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.2), lineWidth: 1))
                }
                .buttonStyle(.plain)
                .focusable(false)
            }
        }
        .padding(8)
        .background(Color.white.opacity(0.03))
        .clipShape(RoundedRectangle(cornerRadius: 7))
    }

    private func lapCompareCard(_ lap: LapSnapshot) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack {
                Text("#\(lap.lapNumber)")
                    .font(.system(size: 10, weight: .bold, design: .monospaced))
                    .foregroundStyle(.orange.opacity(0.8))
                Text(lap.label)
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.5))
                    .lineLimit(1)
            }
            compareMetric("Cost",   formatCost(lap.cost))
            compareMetric("Tokens", formatTokens(lap.totalTokens))
            compareMetric("Calls",  "\(lap.eventCount)")
            compareMetric("Dur",    formatDuration(lap.durationSecs))
            if let model = lap.topModel {
                compareMetric("Model", model)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(8)
        .background(Color.white.opacity(0.04))
        .clipShape(RoundedRectangle(cornerRadius: 6))
    }

    private func compareMetric(_ label: String, _ value: String) -> some View {
        HStack(spacing: 4) {
            Text(label)
                .font(.system(size: 8))
                .foregroundStyle(.white.opacity(0.3))
                .frame(width: 36, alignment: .leading)
            Text(value)
                .font(.system(size: 9, weight: .medium, design: .monospaced))
                .foregroundStyle(.white.opacity(0.7))
                .lineLimit(1)
                .minimumScaleFactor(0.7)
        }
    }

    private func previousLap(for lap: LapSnapshot) -> LapSnapshot? {
        apiClient.laps.first(where: { $0.lapNumber == lap.lapNumber - 1 })
    }

    // MARK: - Export

    private enum ExportFormat { case json, csv }

    private func exportLaps(format: ExportFormat) {
        let (data, filename, type): (Data, String, UTType) = switch format {
        case .json:
            (lapExportJSON(), "lumen-laps.json", .json)
        case .csv:
            (lapExportCSV(), "lumen-laps.csv", .commaSeparatedText)
        }

        Task { @MainActor in
            let panel = NSSavePanel()
            panel.nameFieldStringValue = filename
            panel.allowedContentTypes = [type]
            panel.canCreateDirectories = true
            guard panel.runModal() == .OK, let url = panel.url else { return }
            try? data.write(to: url)
        }
    }

    private func lapExportJSON() -> Data {
        struct Export: Encodable {
            let exported_at: String
            let laps: [LapSnapshot]
            let events: [UsageEvent]
        }
        let formatter = ISO8601DateFormatter()
        let export = Export(
            exported_at: formatter.string(from: Date()),
            laps: apiClient.laps,
            events: apiClient.recentEvents
        )
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return (try? encoder.encode(export)) ?? Data()
    }

    private func lapExportCSV() -> Data {
        var rows = ["lap_number,label,started_at,ended_at,duration_secs,input_tokens,output_tokens,total_tokens,cost_usd,cache_savings_usd,event_count,top_model,tokens_per_min,cost_per_min"]
        for lap in apiClient.laps {
            let model = lap.topModel.map { "\"\($0)\"" } ?? ""
            rows.append([
                "\(lap.lapNumber)",
                "\"\(lap.label)\"",
                lap.startedAt, lap.endedAt,
                String(format: "%.1f", lap.durationSecs),
                "\(lap.inputTokens)", "\(lap.outputTokens)", "\(lap.totalTokens)",
                String(format: "%.6f", lap.cost),
                String(format: "%.6f", lap.cacheSavings),
                "\(lap.eventCount)",
                model,
                String(format: "%.1f", lap.tokensPerMinute),
                String(format: "%.6f", lap.costPerMinute),
            ].joined(separator: ","))
        }
        return rows.joined(separator: "\n").data(using: .utf8) ?? Data()
    }

    // (Action buttons moved into persistentFooter — Lap+Clear are now in the
    //  footer's Monitor-only row; Quit is in the always-visible row.)

    // MARK: - DG CTA

    private func dgCTAReason() -> String? {
        // Spike: any single lap cost > 2× the median lap cost
        let costs = apiClient.laps.map { $0.cost }.filter { $0 > 0 }
        if costs.count >= 2 {
            let sorted = costs.sorted()
            let median = sorted[sorted.count / 2]
            if let maxCost = costs.max(), maxCost > median * 2 {
                return String(format: "A lap spiked to %@ — DataGrout can help reduce costs by 40–60%%", formatCost(maxCost))
            }
        }

        // Trend: total cost growing across laps
        if apiClient.laps.count >= 3 {
            let lapCosts = apiClient.laps.sorted { $0.lapNumber < $1.lapNumber }.map { $0.cost }
            let firstHalf = lapCosts.prefix(lapCosts.count / 2).reduce(0, +)
            let secondHalf = lapCosts.suffix(lapCosts.count / 2).reduce(0, +)
            if secondHalf > firstHalf * 1.3 {
                return String(format: "Your costs are trending up (%@ total) — DataGrout typically cuts this by half", formatCost(apiClient.stats.totalCost))
            }
        }

        // High total: session total > $2
        if apiClient.stats.totalCost > 2.0 {
            return String(format: "You've spent %@ this session — DataGrout can significantly reduce that", formatCost(apiClient.stats.totalCost))
        }

        return nil
    }

    private func dgCTABanner(reason: String) -> some View {
        HStack(spacing: 10) {
            Image(systemName: "sparkles")
                .font(.system(size: 13))
                .foregroundStyle(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.8))

            VStack(alignment: .leading, spacing: 2) {
                Text(reason)
                    .font(.system(size: 10))
                    .foregroundStyle(.white.opacity(0.75))
                    .fixedSize(horizontal: false, vertical: true)
                Text("Connect DataGrout in Settings →")
                    .font(.system(size: 9, weight: .medium))
                    .foregroundStyle(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.7))
            }

            Spacer()

            Button(action: { dgCtaDismissedAt = Date().timeIntervalSince1970 }) {
                Image(systemName: "xmark")
                    .font(.system(size: 8))
                    .foregroundStyle(.white.opacity(0.3))
            }
            .buttonStyle(.plain)
            .focusable(false)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.06))
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color(red: 0.4, green: 0.6, blue: 1.0).opacity(0.18), lineWidth: 1))
    }

    private var lapNamingRow: some View {
        HStack(spacing: 6) {
            TextField("Name this lap… (optional)", text: $lapNameInput)
                .textFieldStyle(.plain)
                .font(.system(size: 11))
                .foregroundStyle(.white.opacity(0.85))
                .focused($lapNameFocused)
                .onSubmit { commitLap() }
                .onExitCommand { cancelLap() }

            Button(action: { commitLap() }) {
                Image(systemName: "checkmark")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.orange.opacity(0.8))
            }
            .buttonStyle(.plain)
            .focusable(false)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(Color.orange.opacity(0.06))
        .clipShape(RoundedRectangle(cornerRadius: 7))
        .overlay(RoundedRectangle(cornerRadius: 7).stroke(Color.orange.opacity(0.25), lineWidth: 1))
    }

    private func commitLap() {
        let label = lapNameInput.trimmingCharacters(in: .whitespaces)
        namingLap = false
        lapNameFocused = false
        lapNameInput = ""
        Task { await apiClient.createLap(label: label.isEmpty ? nil : label) }
    }

    private func cancelLap() {
        namingLap = false
        lapNameFocused = false
        lapNameInput = ""
    }
}

// MARK: - Lap Row

struct LapRow: View {
    let lap: LapSnapshot
    let previousLap: LapSnapshot?
    var isSelected: Bool = false
    var onToggle: (() -> Void)? = nil

    var body: some View {
        Button(action: { onToggle?() }) {
            VStack(alignment: .leading, spacing: 3) {
                HStack {
                    HStack(spacing: 4) {
                        Text("#\(lap.lapNumber)")
                            .font(.system(size: 10, weight: .bold, design: .monospaced))
                            .foregroundStyle(.orange.opacity(0.8))
                        Text(lap.label)
                            .font(.system(size: 10))
                            .foregroundStyle(.white.opacity(0.7))
                            .lineLimit(1)
                    }
                    Spacer()
                    Text(formatDuration(lap.durationSecs))
                        .font(.system(size: 9, design: .monospaced))
                        .foregroundStyle(.white.opacity(0.4))
                }

                HStack(spacing: 8) {
                    lapStat(formatTokens(lap.totalTokens), label: "tok", delta: deltaTokens)
                    lapStat(formatCost(lap.cost), label: "cost", delta: deltaCost)
                    lapStat("\(lap.eventCount)", label: "calls", delta: nil)
                    if let model = lap.topModel {
                        Text(model)
                            .font(.system(size: 8))
                            .foregroundStyle(.white.opacity(0.3))
                            .lineLimit(1)
                    }
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            .background(isSelected ? Color.orange.opacity(0.08) : Color.white.opacity(0.03))
            .clipShape(RoundedRectangle(cornerRadius: 5))
            .overlay(
                RoundedRectangle(cornerRadius: 5)
                    .stroke(isSelected ? Color.orange.opacity(0.4) : Color.clear, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
        .contentShape(Rectangle())
        .focusable(false)
    }

    private var deltaTokens: String? {
        guard let prev = previousLap, prev.totalTokens > 0 else { return nil }
        let pct = Double(lap.totalTokens - prev.totalTokens) / Double(prev.totalTokens) * 100
        return formatDelta(pct)
    }

    private var deltaCost: String? {
        guard let prev = previousLap, prev.cost > 0.0001 else { return nil }
        let pct = (lap.cost - prev.cost) / prev.cost * 100
        return formatDelta(pct)
    }

    private func lapStat(_ value: String, label: String, delta: String?) -> some View {
        HStack(spacing: 2) {
            Text(value)
                .font(.system(size: 10, weight: .medium, design: .monospaced))
                .foregroundStyle(.white.opacity(0.65))
            Text(label)
                .font(.system(size: 8))
                .foregroundStyle(.white.opacity(0.3))
            if let delta = delta {
                Text(delta)
                    .font(.system(size: 9, weight: .semibold, design: .monospaced))
                    .foregroundStyle(delta.hasPrefix("-") ? .green : .red.opacity(0.7))
            }
        }
    }
}

func formatDuration(_ secs: Double) -> String {
    if secs < 60 { return String(format: "%.0fs", secs) }
    let mins = Int(secs) / 60
    let remainder = Int(secs) % 60
    if mins < 60 { return "\(mins)m\(remainder)s" }
    let hours = mins / 60
    return "\(hours)h\(mins % 60)m"
}

func formatDelta(_ pct: Double) -> String {
    if pct >= 0 { return String(format: "+%.0f%%", pct) }
    return String(format: "%.0f%%", pct)
}
