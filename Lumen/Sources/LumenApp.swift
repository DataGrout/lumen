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
            button.action = #selector(togglePopover)
            button.target = self

            statusIconManager = StatusIconManager(button: button, apiClient: apiClient)
        }

        popover = NSPopover()
        popover.contentSize = NSSize(width: 400, height: 640)
        popover.behavior = .transient
        popover.contentViewController = NSHostingController(
            rootView: PopoverView(apiClient: apiClient, daemonManager: daemonManager)
        )

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

    @objc func togglePopover() {
        guard let button = statusItem.button else { return }

        if popover.isShown {
            popover.performClose(nil)
        } else {
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            NSApp.activate(ignoringOtherApps: true)
        }
    }
}
