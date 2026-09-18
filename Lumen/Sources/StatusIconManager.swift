import AppKit

final class StatusIconManager {
    private let button: NSStatusBarButton
    private let apiClient: APIClient
    private var timer: Timer?
    private var pulsePhase = false
    private var lastEventCount = 0

    private enum IconState {
        case disconnected
        /// Nothing is getting through the proxy. Distinct from `disconnected`,
        /// which is the daemon being unreachable — this is the daemon running
        /// fine and failing to carry traffic, which looks identical from the
        /// menu bar and is the more dangerous of the two, because every client
        /// on the machine routes through it.
        case degraded
        case idle
        case active
    }

    init(button: NSStatusBarButton, apiClient: APIClient) {
        self.button = button
        self.apiClient = apiClient
        applyIcon(symbolForState(.disconnected))
        startMonitoring()
    }

    func startMonitoring() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            self?.updateIcon()
        }
        updateIcon()
    }

    func stopMonitoring() {
        timer?.invalidate()
        timer = nil
    }

    private func updateIcon() {
        let state = currentState()
        let symbolName = symbolForState(state)
        let degraded = state == .degraded
        let detail = apiClient.upstreamLastError
        DispatchQueue.main.async { [weak self] in
            self?.applyIcon(symbolName, degraded: degraded, detail: detail)
        }
    }

    private func applyIcon(_ symbolName: String, degraded: Bool = false, detail: String? = nil) {
        guard let image = NSImage(
            systemSymbolName: symbolName,
            accessibilityDescription: "Lumen"
        ) else { return }

        // A template image is tinted to match the menu bar, which is what makes
        // the normal icon unobtrusive — and exactly why it cannot carry a
        // warning. Amber only happens by opting out of template rendering.
        let config: NSImage.SymbolConfiguration = degraded
            ? NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
                .applying(NSImage.SymbolConfiguration(paletteColors: [.systemOrange]))
            : NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)

        let configured = image.withSymbolConfiguration(config) ?? image
        configured.isTemplate = !degraded
        button.image = configured
        button.toolTip = degraded
            ? "Lumen: nothing is getting through — click for options"
                + (detail.map { "\n\($0)" } ?? "")
            : "Lumen by DataGrout"
    }

    private func currentState() -> IconState {
        if !apiClient.connected {
            return .disconnected
        }

        // Ahead of the activity states: a proxy that is busy failing still
        // increments counters, and "active" over a broken path is a lie.
        if apiClient.upstreamDegraded {
            return .degraded
        }

        let newEvents = apiClient.stats.eventCount > lastEventCount
        if newEvents {
            lastEventCount = apiClient.stats.eventCount
            pulsePhase.toggle()
            return .active
        }

        return .idle
    }

    private func symbolForState(_ state: IconState) -> String {
        switch state {
        case .disconnected:
            return "gauge.open.with.lines.needle.0percent"
        case .degraded:
            return "exclamationmark.triangle.fill"
        case .idle:
            return "gauge.open.with.lines.needle.33percent"
        case .active:
            return pulsePhase
                ? "gauge.open.with.lines.needle.67percent"
                : "arrow.clockwise"
        }
    }
}
