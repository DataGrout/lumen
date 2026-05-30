import Foundation
import AppKit

/// Single source of truth for spawning LLM clients with the right
/// proxy / env-var configuration. Used by both the Launch tab UI and the
/// status-bar right-click menu — having them call into this struct prevents
/// the two call sites from drifting as launchers are added or tweaked.
enum LumenLauncher: String, CaseIterable, Identifiable {
    case claudeCode    = "claude-code"
    case opencode
    case cursor
    case claudeDesktop = "claude-desktop"

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .claudeCode:    return "Claude Code"
        case .opencode:      return "OpenCode"
        case .cursor:        return "Cursor"
        case .claudeDesktop: return "Claude Desktop"
        }
    }

    var mode: Mode {
        switch self {
        case .claudeCode, .opencode: return .relay
        case .cursor, .claudeDesktop: return .proxy
        }
    }

    enum Mode {
        case relay   // env-var redirect, no CA trust needed
        case proxy   // HTTPS_PROXY + NODE_EXTRA_CA_CERTS, requires CA trust
    }

    /// Whether the prerequisite binary / .app is present on disk. Cached at
    /// call-site, not here, so the menu can decide whether to dim items.
    var available: Bool {
        switch self {
        case .claudeCode:    return LauncherSupport.findBinary("claude") != nil
        case .opencode:      return LauncherSupport.findBinary("opencode") != nil
        case .cursor:        return LauncherSupport.appExists("Cursor")
        case .claudeDesktop: return LauncherSupport.appExists("Claude")
        }
    }

    /// Fire-and-forget launch. Calls `completion` on the main queue once the
    /// spawn has been initiated (not when the launched app finishes loading).
    func launch(proxyPort: Int, caPath: String, completion: @escaping () -> Void = {}) {
        DispatchQueue.global(qos: .userInitiated).async {
            switch self {
            case .claudeCode:    LauncherSupport.spawnRelayTerminal(port: proxyPort, command: "claude")
            case .opencode:      LauncherSupport.spawnRelayTerminal(port: proxyPort, command: "opencode")
            case .cursor:
                LauncherSupport.spawnProxyApp(
                    binary: "/Applications/Cursor.app/Contents/MacOS/Cursor",
                    port: proxyPort,
                    caPath: caPath
                )
            case .claudeDesktop:
                LauncherSupport.spawnProxyApp(
                    binary: "/Applications/Claude.app/Contents/MacOS/Claude",
                    port: proxyPort,
                    caPath: caPath
                )
            }
            DispatchQueue.main.async(execute: completion)
        }
    }
}

/// Helpers shared by the launcher enum. Kept namespaced so they don't leak
/// into general autocomplete.
enum LauncherSupport {
    static func findBinary(_ name: String) -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let candidates = [
            "/usr/local/bin/\(name)",
            "/opt/homebrew/bin/\(name)",
            "/usr/bin/\(name)",
            "\(home)/.local/bin/\(name)",
            "\(home)/.npm-global/bin/\(name)",
        ]
        return candidates.first { FileManager.default.fileExists(atPath: $0) }
    }

    static func appExists(_ name: String) -> Bool {
        FileManager.default.fileExists(atPath: "/Applications/\(name).app")
    }

    static func resolvedCAPath(from caInfoPath: String?) -> String {
        caInfoPath ?? NSString("~/.lumen/ca.pem").expandingTildeInPath
    }

    /// Spawn a Terminal window with `ANTHROPIC_BASE_URL` set so the user can
    /// `claude` / `opencode` straight away. Relay mode — no CA trust needed.
    static func spawnRelayTerminal(port: Int, command: String) {
        let baseURL = "http://127.0.0.1:\(port)/anthropic"
        let script = """
        tell application "Terminal"
            activate
            do script "export ANTHROPIC_BASE_URL=\(baseURL) && echo '✓ Lumen relay active — run: \(command)'"
        end tell
        """
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        proc.arguments = ["-e", script]
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        try? proc.run()
        proc.waitUntilExit()
    }

    /// Spawn a proxy-mode GUI app with HTTPS_PROXY + NODE_EXTRA_CA_CERTS set.
    static func spawnProxyApp(binary: String, port: Int, caPath: String) {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: binary)
        proc.arguments = ["--proxy-server=http://127.0.0.1:\(port)"]
        var env = ProcessInfo.processInfo.environment
        env["HTTPS_PROXY"] = "http://127.0.0.1:\(port)"
        env["HTTP_PROXY"]  = "http://127.0.0.1:\(port)"
        env["NODE_EXTRA_CA_CERTS"] = caPath
        proc.environment = env
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        try? proc.run()
    }
}
