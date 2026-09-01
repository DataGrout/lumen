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
                        id: LumenLauncher.claudeCode.rawValue,
                        name: LumenLauncher.claudeCode.displayName,
                        icon: "apple.terminal",
                        mode: .relay,
                        available: LumenLauncher.claudeCode.available,
                        action: launchClaudeCode,
                        setupNote: "No CA trust needed — relay injects the base URL via env var.",
                        setupCommand: "ANTHROPIC_BASE_URL=http://127.0.0.1:\(apiClient.proxyConfig.port)/anthropic claude"
                    )
                    card(
                        id: LumenLauncher.cursor.rawValue,
                        name: LumenLauncher.cursor.displayName,
                        icon: "curlybraces",
                        mode: .proxy,
                        available: LumenLauncher.cursor.available,
                        action: launchCursor,
                        setupNote: "① Trust Lumen CA — Settings → Certificate\n② Cursor Settings → Network → HTTP Compatibility → HTTP/1.1\n③ Click Launch",
                        setupCommand: cursorLaunchCommand
                    )
                    card(
                        id: LumenLauncher.claudeDesktop.rawValue,
                        name: LumenLauncher.claudeDesktop.displayName,
                        icon: "bubble.left.and.bubble.right",
                        mode: .proxy,
                        available: LumenLauncher.claudeDesktop.available,
                        action: launchClaudeDesktop,
                        setupNote: "Trust Lumen CA in Settings → Certificate, then click Launch.",
                        setupCommand: nil
                    )
                    card(
                        id: LumenLauncher.opencode.rawValue,
                        name: LumenLauncher.opencode.displayName,
                        icon: "hammer",
                        mode: .relay,
                        available: LumenLauncher.opencode.available,
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

    /// All four launchers funnel through LumenLauncher (LaunchService.swift)
    /// — the spinner / state management lives here, the actual spawn logic
    /// lives in one place. Keeps this view and the right-click menu in sync.
    private func launch(_ launcher: LumenLauncher) {
        launching = launcher.rawValue
        let port = apiClient.proxyConfig.port
        let caPath = LauncherSupport.resolvedCAPath(from: apiClient.caInfo?.path)
        // Relay-mode launchers settle in ~1s (Terminal handoff), proxy-mode
        // ones take ~2s (full .app cold-start). We approximate so the spinner
        // doesn't linger.
        let settleDelay: TimeInterval = launcher.mode == .relay ? 1 : 2

        launcher.launch(proxyPort: port, caPath: caPath) {
            DispatchQueue.main.asyncAfter(deadline: .now() + settleDelay) {
                self.launching = nil
                NotificationCenter.default.post(
                    name: .lumenShowTab,
                    object: AppTab.monitor.rawValue
                )
            }
        }
    }

    private func launchClaudeCode()    { launch(.claudeCode) }
    private func launchOpenCode()      { launch(.opencode) }
    private func launchCursor()        { launch(.cursor) }
    private func launchClaudeDesktop() { launch(.claudeDesktop) }

    // MARK: - Helpers

    /// The copyable manual-launch command shown in Cursor's info expansion.
    /// Generated locally because it references the current proxy port and
    /// resolved CA path; the actual launch goes through LumenLauncher.
    private var cursorLaunchCommand: String {
        let port = apiClient.proxyConfig.port
        let caPath = LauncherSupport.resolvedCAPath(from: apiClient.caInfo?.path)
        return "HTTPS_PROXY=http://127.0.0.1:\(port) NODE_EXTRA_CA_CERTS=\(caPath) /Applications/Cursor.app/Contents/MacOS/Cursor --proxy-server=http://127.0.0.1:\(port)"
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
            Button(action: { daemonManager.resetFailures(); daemonManager.start() }) {
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
