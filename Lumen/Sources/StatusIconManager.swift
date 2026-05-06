import AppKit

final class StatusIconManager {
    private let button: NSStatusBarButton
    private let apiClient: APIClient
    private var timer: Timer?
    private var pulsePhase = false
    private var lastEventCount = 0

    private enum IconState {
        case disconnected
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
        DispatchQueue.main.async { [weak self] in
            self?.applyIcon(symbolName)
        }
    }

    private func applyIcon(_ symbolName: String) {
        guard let image = NSImage(
            systemSymbolName: symbolName,
            accessibilityDescription: "Lumen"
        ) else { return }

        let config = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        let configured = image.withSymbolConfiguration(config) ?? image
        configured.isTemplate = true
        button.image = configured
        button.toolTip = "Lumen by DataGrout"
    }

    private func currentState() -> IconState {
        if !apiClient.connected {
            return .disconnected
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
        case .idle:
            return "gauge.open.with.lines.needle.33percent"
        case .active:
            return pulsePhase
                ? "gauge.open.with.lines.needle.67percent"
                : "arrow.clockwise"
        }
    }
}
