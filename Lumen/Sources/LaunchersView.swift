import SwiftUI
import AppKit

struct LaunchersView: View {
    let apiClient: APIClient
    let daemonManager: DaemonManager

    @State private var launching: String? = nil
    @State private var expandedSetup: Set<String> = []

    var body: some View {
        ScrollView {
            VStack(spacing: 10) {
                if !apiClient.connected {
                    daemonWarning
                }

                LazyVGrid(
                    columns: [GridItem(.flexible()), GridItem(.flexible())],
                    spacing: 10
                ) {
                    card(
                        id: "claude-code",
                        name: "Claude Code",
                        icon: "apple.terminal",
                        mode: .relay,
                        available: findBinary("claude") != nil,
                        action: launchClaudeCode,
                        setupNote: "No CA trust needed — relay injects the base URL via env var.",
                        setupCommand: "ANTHROPIC_BASE_URL=http://127.0.0.1:\(apiClient.proxyConfig.port)/anthropic claude"
                    )
                    card(
                        id: "cursor",
                        name: "Cursor",
                        icon: "curlybraces",
                        mode: .proxy,
                        available: appExists("Cursor"),
                        action: launchCursor,
                        setupNote: "① Trust Lumen CA — Settings → Certificate\n② Cursor Settings → Network → HTTP Compatibility → HTTP/1.1\n③ Click Launch",
                        setupCommand: cursorLaunchCommand
                    )
                    card(
                        id: "claude-desktop",
                        name: "Claude Desktop",
                        icon: "bubble.left.and.bubble.right",
                        mode: .proxy,
                        available: appExists("Claude"),
                        action: launchClaudeDesktop,
                        setupNote: "Trust Lumen CA in Settings → Certificate, then click Launch.",
                        setupCommand: nil
                    )
                    card(
                        id: "opencode",
                        name: "OpenCode",
                        icon: "hammer",
                        mode: .relay,
                        available: findBinary("opencode") != nil,
                        action: launchOpenCode,
                        setupNote: "No CA trust needed — relay injects the base URL via env var.",
                        setupCommand: "ANTHROPIC_BASE_URL=http://127.0.0.1:\(apiClient.proxyConfig.port)/anthropic opencode"
                    )
                }

                proxyNote
            }
            .padding(.vertical, 4)
        }
    }

    // MARK: - Card

    private enum Mode {
        case relay, proxy

        var label: String { self == .relay ? "relay" : "proxy" }
        var color: Color { self == .relay ? .teal : Color(red: 0.4, green: 0.6, blue: 1.0) }
        var detail: String { self == .relay ? "No cert needed" : "CA cert required" }
    }

    private func card(
        id: String,
        name: String,
        icon: String,
        mode: Mode,
        available: Bool,
        action: @escaping () -> Void,
        setupNote: String?,
        setupCommand: String?
    ) -> some View {
        let isExpanded = expandedSetup.contains(id)

        return VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top) {
                Image(systemName: icon)
                    .font(.system(size: 20))
                    .foregroundStyle(available ? .orange : .white.opacity(0.2))
                Spacer()
                Text(mode.label)
                    .font(.system(size: 8, weight: .semibold))
                    .textCase(.uppercase)
                    .tracking(0.4)
                    .foregroundStyle(mode.color.opacity(0.85))
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(mode.color.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 3))
            }
            .padding(.bottom, 8)

            Text(name)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.white.opacity(available ? 0.9 : 0.35))
                .lineLimit(1)
                .minimumScaleFactor(0.8)

            Text(available ? mode.detail : "Not installed")
                .font(.system(size: 9))
                .foregroundStyle(.white.opacity(0.3))
                .padding(.top, 2)

            Spacer(minLength: 8)

            HStack(spacing: 5) {
                Button(action: {
                    guard launching == nil, available else { return }
                    action()
                }) {
                    HStack(spacing: 5) {
                        if launching == id {
                            ProgressView()
                                .scaleEffect(0.5)
                                .frame(width: 10, height: 10)
                        }
                        Text("Launch")
                            .font(.system(size: 10, weight: .semibold))
                    }
                    .foregroundStyle(available ? .black : .white.opacity(0.2))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 7)
                    .background(available ? Color.orange : Color.white.opacity(0.04))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
                .buttonStyle(.plain)
                .focusable(false)
                .disabled(!available || launching != nil)

                if setupNote != nil || setupCommand != nil {
                    Button(action: {
                        if isExpanded { expandedSetup.remove(id) }
                        else { expandedSetup.insert(id) }
                    }) {
                        Image(systemName: isExpanded ? "info.circle.fill" : "info.circle")
                            .font(.system(size: 18))
                            .foregroundStyle(isExpanded ? .orange.opacity(0.7) : .white.opacity(0.22))
                    }
                    .buttonStyle(.plain)
                    .focusable(false)
                }
            }
            .padding(.top, 10)

            if isExpanded {
                VStack(alignment: .leading, spacing: 6) {
                    Divider()
                        .background(Color.white.opacity(0.08))
                        .padding(.vertical, 2)

                    if let note = setupNote {
                        Text(note)
                            .font(.system(size: 9))
                            .foregroundStyle(.white.opacity(0.45))
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let cmd = setupCommand {
                        setupCodeRow(cmd)
                    }
                }
                .padding(.top, 4)
            }
        }
        .frame(minHeight: 130)
        .padding(12)
        .background(Color.white.opacity(0.04))
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(Color.white.opacity(0.07), lineWidth: 1)
        )
    }

    private func setupCodeRow(_ text: String) -> some View {
        HStack(spacing: 4) {
            Text(text)
                .font(.system(size: 8, design: .monospaced))
                .foregroundStyle(.orange.opacity(0.7))
                .lineLimit(3)
                .minimumScaleFactor(0.7)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 4)
            Button(action: {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
            }) {
                Image(systemName: "doc.on.doc")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.35))
            }
            .buttonStyle(.plain)
            .focusable(false)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(Color.white.opacity(0.05))
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }

    // MARK: - Launch Actions

    private func launchClaudeCode() {
        launching = "claude-code"
        let port = apiClient.proxyConfig.port
        let baseURL = "http://127.0.0.1:\(port)/anthropic"

        DispatchQueue.global(qos: .userInitiated).async {
            let script = """
            tell application "Terminal"
                activate
                do script "export ANTHROPIC_BASE_URL=\(baseURL) && echo '✓ Lumen relay active — run: claude'"
            end tell
            """
            runAppleScript(script)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { launching = nil }
        }
    }

    private func launchOpenCode() {
        launching = "opencode"
        let port = apiClient.proxyConfig.port
        let baseURL = "http://127.0.0.1:\(port)/anthropic"

        DispatchQueue.global(qos: .userInitiated).async {
            let script = """
            tell application "Terminal"
                activate
                do script "export ANTHROPIC_BASE_URL=\(baseURL) && echo '✓ Lumen relay active — run: opencode'"
            end tell
            """
            runAppleScript(script)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { launching = nil }
        }
    }

    private func launchCursor() {
        launching = "cursor"
        let port = apiClient.proxyConfig.port
        let caPath = resolvedCAPath()

        DispatchQueue.global(qos: .userInitiated).async {
            launchApp(
                binary: "/Applications/Cursor.app/Contents/MacOS/Cursor",
                args: ["--proxy-server=http://127.0.0.1:\(port)"],
                extraEnv: proxyEnv(port: port, caPath: caPath)
            )
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { launching = nil }
        }
    }

    private func launchClaudeDesktop() {
        launching = "claude-desktop"
        let port = apiClient.proxyConfig.port
        let caPath = resolvedCAPath()

        DispatchQueue.global(qos: .userInitiated).async {
            launchApp(
                binary: "/Applications/Claude.app/Contents/MacOS/Claude",
                args: ["--proxy-server=http://127.0.0.1:\(port)"],
                extraEnv: proxyEnv(port: port, caPath: caPath)
            )
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { launching = nil }
        }
    }

    // MARK: - Helpers

    private var cursorLaunchCommand: String {
        let port = apiClient.proxyConfig.port
        let caPath = resolvedCAPath()
        return "HTTPS_PROXY=http://127.0.0.1:\(port) NODE_EXTRA_CA_CERTS=\(caPath) /Applications/Cursor.app/Contents/MacOS/Cursor --proxy-server=http://127.0.0.1:\(port)"
    }

    private func appExists(_ name: String) -> Bool {
        FileManager.default.fileExists(atPath: "/Applications/\(name).app")
    }

    private func findBinary(_ name: String) -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let candidates = [
            "/usr/local/bin/\(name)",
            "/opt/homebrew/bin/\(name)",
            "/usr/bin/\(name)",
            "\(home)/.local/bin/\(name)",
            "\(home)/.npm-global/bin/\(name)"
        ]
        return candidates.first { FileManager.default.fileExists(atPath: $0) }
    }

    private func resolvedCAPath() -> String {
        apiClient.caInfo?.path
            ?? NSString("~/.lumen/ca.pem").expandingTildeInPath
    }

    private func proxyEnv(port: Int, caPath: String) -> [String: String] {
        [
            "HTTPS_PROXY": "http://127.0.0.1:\(port)",
            "HTTP_PROXY": "http://127.0.0.1:\(port)",
            "NODE_EXTRA_CA_CERTS": caPath
        ]
    }

    private func launchApp(binary: String, args: [String], extraEnv: [String: String]) {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: binary)
        proc.arguments = args
        var env = ProcessInfo.processInfo.environment
        extraEnv.forEach { env[$0] = $1 }
        proc.environment = env
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        try? proc.run()
    }

    private func runAppleScript(_ script: String) {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        proc.arguments = ["-e", script]
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        try? proc.run()
        proc.waitUntilExit()
    }

    // MARK: - Banners

    private var daemonWarning: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 11))
                .foregroundStyle(.red.opacity(0.8))
            Text("Daemon not running — tracking inactive")
                .font(.system(size: 9))
                .foregroundStyle(.white.opacity(0.55))
            Spacer()
            Button(action: { daemonManager.start() }) {
                Text("Restart")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.red.opacity(0.9))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Color.red.opacity(0.1))
                    .clipShape(RoundedRectangle(cornerRadius: 4))
                    .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.red.opacity(0.3), lineWidth: 1))
            }
            .buttonStyle(.plain)
            .focusable(false)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(Color.red.opacity(0.07))
        .clipShape(RoundedRectangle(cornerRadius: 7))
        .overlay(RoundedRectangle(cornerRadius: 7).stroke(Color.red.opacity(0.2), lineWidth: 1))
    }

    private var proxyNote: some View {
        HStack(spacing: 5) {
            Image(systemName: "lock.shield")
                .font(.system(size: 9))
            Text("Proxy-mode apps (Cursor, Claude Desktop) require CA trust. See Settings → Certificate.")
                .font(.system(size: 9))
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(.white.opacity(0.25))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 2)
    }
}
