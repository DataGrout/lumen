import AppKit
import SwiftUI

@main
struct LumenApp {
    static func main() {
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.run()
    }
}

class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var statusItem: NSStatusItem!
    var popover: NSPopover!
    var apiClient: APIClient!
    var daemonManager: DaemonManager!
    var statusIconManager: StatusIconManager!
    var wizardWindow: NSWindow?
    /// Global mouse-down monitor used to dismiss the popover when clicking outside.
    private var clickOutsideMonitor: Any?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        daemonManager = DaemonManager()
        daemonManager.start()

        DispatchQueue.global(qos: .background).async { [weak self] in
            let healthy = self?.daemonManager.waitForHealthy(timeout: 5.0) ?? false
            NSLog("[Lumen] Daemon health check: %@", healthy ? "ready" : "timed out")

            guard healthy, UserDefaults.standard.bool(forKey: "lumen.autoEnableProxy") else { return }
            let savedPort = UserDefaults.standard.integer(forKey: "lumen.proxyPort")
            let port = savedPort > 0 ? savedPort : 9090
            let iface = SystemProxy.activeInterface()
            Task {
                let ok = await SystemProxy.enable(port: port, interface: iface)
                NSLog("[Lumen] Auto-restored system proxy on port %d: %@", port, ok ? "ok" : "failed")
            }
        }

        apiClient = APIClient()

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)

        if let button = statusItem.button {
            button.image = NSImage(
                systemSymbolName: "gauge.open.with.lines.needle.33percent",
                accessibilityDescription: "Lumen"
            )
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
            button.action = #selector(handleStatusBarClick)
            button.target = self

            statusIconManager = StatusIconManager(button: button, apiClient: apiClient)
        }

        popover = NSPopover()
        popover.contentSize = NSSize(width: 400, height: 640)
        popover.behavior = .transient
        popover.contentViewController = NSHostingController(
            rootView: PopoverView(apiClient: apiClient, daemonManager: daemonManager)
        )

        // Let the setup wizard (a separate window) route the user into the
        // popover's Settings tab: close the wizard, open the popover, switch tab.
        NotificationCenter.default.addObserver(
            forName: .lumenOpenSettings, object: nil, queue: .main
        ) { [weak self] _ in
            guard let self, let button = self.statusItem.button else { return }
            self.wizardWindow?.close()
            if !self.popover.isShown { self.openPopover(relativeTo: button) }
            NotificationCenter.default.post(name: .lumenShowTab, object: AppTab.settings.rawValue)
        }

        if !UserDefaults.standard.bool(forKey: "lumen.suppressLauncher") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                let setupComplete = UserDefaults.standard.bool(forKey: "lumen.setupComplete")
                self.showWizard(startAtDone: setupComplete)
            }
        }
    }

    private func showWizard(startAtDone: Bool = false) {
        let wizardView = WizardView(apiClient: apiClient, startAtDone: startAtDone) { [weak self] in
            UserDefaults.standard.set(true, forKey: "lumen.setupComplete")
            self?.wizardWindow?.close()
            self?.wizardWindow = nil
        }

        let controller = NSHostingController(rootView: wizardView)
        let window = NSWindow(contentViewController: controller)
        window.title = "Lumen Setup"
        window.styleMask = [.titled, .closable, .fullSizeContentView]
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = true
        window.center()
        window.isReleasedWhenClosed = false
        window.delegate = self
        // Show in Cmd+Tab while the wizard is open
        NSApp.setActivationPolicy(.regular)
        NSApp.applicationIconImage = loadAppIcon()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        wizardWindow = window
    }

    func applicationWillTerminate(_ notification: Notification) {
        disableSystemProxyIfActive()
        daemonManager.stop()
    }

    private func disableSystemProxyIfActive() {
        let iface = SystemProxy.activeInterface()
        if SystemProxy.isEnabled(interface: iface) {
            NSLog("[Lumen] Disabling system proxy on quit")
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
            proc.arguments = ["-setwebproxystate", iface, "off"]
            try? proc.run()
            proc.waitUntilExit()

            let proc2 = Process()
            proc2.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
            proc2.arguments = ["-setsecurewebproxystate", iface, "off"]
            try? proc2.run()
            proc2.waitUntilExit()
        }
    }

    // MARK: - NSWindowDelegate

    func windowWillClose(_ notification: Notification) {
        guard (notification.object as? NSWindow) === wizardWindow else { return }
        UserDefaults.standard.set(true, forKey: "lumen.setupComplete")
        wizardWindow = nil
        NSApp.setActivationPolicy(.accessory)
    }

    private func loadAppIcon() -> NSImage? {
        // In a proper .app bundle
        if let icon = Bundle.main.image(forResource: "AppIcon") { return icon }
        // In dev build: binary at Lumen/.build/debug/Lumen, icon at Lumen/Resources/AppIcon.icns
        if let exe = Bundle.main.executablePath {
            let icns = URL(fileURLWithPath: exe)
                .deletingLastPathComponent()
                .deletingLastPathComponent()
                .deletingLastPathComponent()
                .appendingPathComponent("Resources/AppIcon.icns")
            if let img = NSImage(contentsOf: icns) { return img }
        }
        return nil
    }

    // MARK: - Status bar click handling

    @objc func handleStatusBarClick() {
        guard let event = NSApp.currentEvent else { return }
        if event.type == .rightMouseUp {
            showContextMenu()
        } else {
            togglePopover()
        }
    }

    @objc func togglePopover() {
        guard let button = statusItem.button else { return }

        if popover.isShown {
            closePopover()
        } else {
            openPopover(relativeTo: button)
        }
    }

    private func openPopover(relativeTo button: NSButton) {
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        // Make the popover's window key so the first click hits content
        // immediately. With LSUIElement = true (accessory app) and .transient
        // behavior (no NSApp.activate), AppKit doesn't auto-promote the
        // popover to key window — symptom was: click a tab once → nothing
        // happens (event consumed by focus grab) → click again → tab switches.
        // makeKey() (vs. makeKeyAndOrderFront / NSApp.activate) doesn't steal
        // focus from the user's foreground app, so the popover behaves like a
        // proper menu rather than yanking the system focus.
        popover.contentViewController?.view.window?.makeKey()
        // Belt-and-suspenders: close on any click outside (complements .transient behavior)
        clickOutsideMonitor = NSEvent.addGlobalMonitorForEvents(
            matching: [.leftMouseDown, .rightMouseDown]
        ) { [weak self] _ in
            self?.closePopover()
        }
    }

    private func closePopover() {
        popover.performClose(nil)
        if let monitor = clickOutsideMonitor {
            NSEvent.removeMonitor(monitor)
            clickOutsideMonitor = nil
        }
    }

    // MARK: - Right-click context menu

    private func showContextMenu() {
        let menu = NSMenu()

        // Version header (disabled). Lead with the daemon version (always
        // present once connected, even in dev). Append the app-bundle version
        // only when it's known AND differs — a mismatch usually means an older
        // daemon is attached.
        let core = apiClient.coreVersion
        let app = LumenVersion.appBundle
        let primary = core ?? app ?? "unknown"
        let suffix: String = {
            if let core, let app, core != app { return "  (app \(app))" }
            return ""
        }()
        let versionItem = NSMenuItem(title: "Lumen \(primary)\(suffix)", action: nil, keyEquivalent: "")
        versionItem.isEnabled = false
        menu.addItem(versionItem)
        menu.addItem(.separator())

        // Spending summary
        let lapCost = apiClient.stats.sessionCost
        let totalCost = apiClient.stats.totalCost
        let lapLabel = String(format: "Lap: $%.4f  •  Total: $%.2f", lapCost, totalCost)
        let summaryItem = NSMenuItem(title: lapLabel, action: nil, keyEquivalent: "")
        summaryItem.isEnabled = false
        menu.addItem(summaryItem)

        menu.addItem(.separator())

        // New lap
        let lapItem = NSMenuItem(title: "New Lap", action: #selector(newLap), keyEquivalent: "")
        lapItem.target = self
        menu.addItem(lapItem)

        menu.addItem(.separator())

        // Tab navigation
        let monitorItem = NSMenuItem(title: "Monitor", action: #selector(showMonitorTab), keyEquivalent: "")
        monitorItem.target = self
        menu.addItem(monitorItem)

        let settingsItem = NSMenuItem(title: "Settings", action: #selector(showSettingsTab), keyEquivalent: "")
        settingsItem.target = self
        menu.addItem(settingsItem)

        menu.addItem(.separator())

        // Launch submenu
        let launchMenu = NSMenu()
        let claudeCodeItem = NSMenuItem(title: "Claude Code", action: #selector(launchClaudeCode), keyEquivalent: "")
        claudeCodeItem.target = self
        launchMenu.addItem(claudeCodeItem)

        let claudeDesktopItem = NSMenuItem(title: "Claude Desktop", action: #selector(launchClaudeDesktop), keyEquivalent: "")
        claudeDesktopItem.target = self
        launchMenu.addItem(claudeDesktopItem)

        let cursorItem = NSMenuItem(title: "Cursor", action: #selector(launchCursor), keyEquivalent: "")
        cursorItem.target = self
        launchMenu.addItem(cursorItem)

        let openCodeItem = NSMenuItem(title: "OpenCode", action: #selector(launchOpenCode), keyEquivalent: "")
        openCodeItem.target = self
        launchMenu.addItem(openCodeItem)

        let launchParent = NSMenuItem(title: "Launch…", action: nil, keyEquivalent: "")
        launchParent.submenu = launchMenu
        menu.addItem(launchParent)

        menu.addItem(.separator())

        // Open the daemon's web dashboard in the user's default browser.
        // Useful as a cross-platform fallback view, or just for the user who
        // wants the data in a real browser tab instead of a status popover.
        let dashboardItem = NSMenuItem(title: "Open Dashboard in Browser",
                                       action: #selector(openDashboard),
                                       keyEquivalent: "")
        dashboardItem.target = self
        menu.addItem(dashboardItem)

        menu.addItem(.separator())

        // TLS trust status (read-only indicator). Reads from APIClient's
        // cached value so the menu opens instantly — the actual subprocess
        // check happens on a background queue in APIClient.refreshCATrust().
        let certTitle = apiClient.caTrusted ? "TLS: Trusted ✓" : "TLS: Not Trusted ⚠"
        let certItem = NSMenuItem(title: certTitle, action: nil, keyEquivalent: "")
        certItem.isEnabled = false
        menu.addItem(certItem)

        menu.addItem(.separator())

        // Quit
        let quitItem = NSMenuItem(title: "Quit Lumen", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        menu.addItem(quitItem)

        if let button = statusItem.button {
            menu.popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.height + 4), in: button)
        }
    }

    @objc private func newLap() {
        Task { await apiClient.createLap() }
    }

    @objc private func showMonitorTab() {
        NotificationCenter.default.post(name: .lumenShowTab, object: AppTab.monitor.rawValue)
        if !popover.isShown, let button = statusItem.button { openPopover(relativeTo: button) }
    }

    @objc private func showSettingsTab() {
        NotificationCenter.default.post(name: .lumenShowTab, object: AppTab.settings.rawValue)
        if !popover.isShown, let button = statusItem.button { openPopover(relativeTo: button) }
    }

    // All four launchers go through LumenLauncher (LaunchService.swift) so
    // this menu and the Launch tab UI share one implementation.
    @objc private func launchClaudeCode()    { runLauncher(.claudeCode) }
    @objc private func launchClaudeDesktop() { runLauncher(.claudeDesktop) }
    @objc private func launchCursor()        { runLauncher(.cursor) }
    @objc private func launchOpenCode()      { runLauncher(.opencode) }

    @objc private func openDashboard() {
        if let url = URL(string: "http://127.0.0.1:9091/dashboard") {
            NSWorkspace.shared.open(url)
        }
    }

    private func runLauncher(_ launcher: LumenLauncher) {
        let port = apiClient.proxyConfig.port
        let caPath = LauncherSupport.resolvedCAPath(from: apiClient.caInfo?.path)
        launcher.launch(proxyPort: port, caPath: caPath) {
            // Surface live activity by jumping the popover to the Monitor tab
            // — same behavior as the Launch tab button.
            NotificationCenter.default.post(
                name: .lumenShowTab,
                object: AppTab.monitor.rawValue
            )
        }
    }
}

// MARK: - Notification Names

extension Notification.Name {
    /// Switch the popover to a specific tab. The notification's `object` is
    /// the `AppTab.rawValue` to activate (e.g. "monitor", "settings"). Posted
    /// by the status-bar right-click menu and by launcher actions.
    static let lumenShowTab = Notification.Name("lumen.showTab")
    /// Open the popover and jump straight to Settings. Posted by the setup
    /// wizard so a user can reach Settings without hunting for the menu icon.
    static let lumenOpenSettings = Notification.Name("lumen.openSettings")
}
