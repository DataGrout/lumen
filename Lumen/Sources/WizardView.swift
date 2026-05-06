import SwiftUI
import AppKit

private enum WizardStep: Int, CaseIterable { case welcome, certificate, captureMethod, done }
private enum CaptureMethod { case proxy, system }

struct WizardView: View {
    let apiClient: APIClient
    let onComplete: () -> Void
    let startAtDone: Bool
    /// Captured once at init — prevents @Observable APIClient from re-rendering
    /// this view on every 2-second poll.
    private let proxyPort: Int

    @State private var step: WizardStep
    @State private var caTrusted = false
    @State private var caInstallBusy = false
    @State private var caInstallError: String? = nil
    @State private var captureMethod: CaptureMethod = .proxy
    @State private var systemProxyBusy = false
    @State private var systemProxyDone = false
    @State private var selectedToolId = ""
    @State private var showMoreTools = false
    /// Installed status per tool id — computed async on appear, never during render.
    @State private var installedTools: Set<String> = []
    @AppStorage("lumen.suppressLauncher") private var suppressOnStartup = false

    private let bg = Color(nsColor: NSColor(red: 0.04, green: 0.04, blue: 0.06, alpha: 1))

    init(apiClient: APIClient, startAtDone: Bool = false, onComplete: @escaping () -> Void) {
        self.apiClient = apiClient
        self.startAtDone = startAtDone
        self.onComplete = onComplete
        self.proxyPort = apiClient.proxyConfig.port
        self._step = State(initialValue: startAtDone ? .done : .welcome)
    }

    var body: some View {
        ZStack {
            bg.ignoresSafeArea()

            VStack(spacing: 0) {
                progressBar
                    .padding(.top, 28)

                Spacer()

                stepContent
                    .padding(.horizontal, 40)

                Spacer()

                navigationRow
                    .padding(.horizontal, 40)
                    .padding(.bottom, 32)
            }
        }
        .frame(width: 460, height: 520)
        .onAppear {
            checkCATrust()
            Task.detached(priority: .utility) {
                let ids = Set(buildAIToolDefs().filter { checkInstalled($0) }.map(\.id))
                await MainActor.run { installedTools = ids }
            }
        }
    }

    // MARK: - Progress bar

    private var progressBar: some View {
        HStack(spacing: 8) {
            ForEach(WizardStep.allCases, id: \.rawValue) { s in
                Capsule()
                    .fill(s.rawValue <= step.rawValue ? Color.orange : Color.white.opacity(0.12))
                    .frame(width: s == step ? 24 : 16, height: 4)
                    .animation(.easeInOut(duration: 0.25), value: step)
            }
        }
        .opacity(startAtDone ? 0 : 1)
    }

    // MARK: - Step content

    @ViewBuilder
    private var stepContent: some View {
        switch step {
        case .welcome:       welcomeStep
        case .certificate:   certificateStep
        case .captureMethod: captureMethodStep
        case .done:          doneStep
        }
    }

    // MARK: Welcome

    private var welcomeStep: some View {
        VStack(spacing: 20) {
            // App icon — use the .icns if bundled, fall back to SF symbol
            Group {
                if let appIcon = NSImage(named: "AppIcon") {
                    Image(nsImage: appIcon)
                        .resizable()
                        .frame(width: 80, height: 80)
                        .clipShape(RoundedRectangle(cornerRadius: 18))
                } else {
                    Image(systemName: "gauge.open.with.lines.needle.67percent")
                        .font(.system(size: 52, weight: .light))
                        .foregroundStyle(.orange)
                        .shadow(color: .orange.opacity(0.4), radius: 20)
                }
            }

            VStack(spacing: 6) {
                Text("Lumen")
                    .font(.system(size: 28, weight: .bold))
                    .foregroundStyle(.white)
                Text("Monitor LLM token usage and costs in real time.\nSetup takes about a minute.")
                    .font(.system(size: 13))
                    .foregroundStyle(.white.opacity(0.5))
                    .multilineTextAlignment(.center)
                    .lineSpacing(3)
            }

            // DataGrout attribution
            Button(action: {
                if let url = URL(string: "https://datagrout.ai") {
                    NSWorkspace.shared.open(url)
                }
            }) {
                HStack(spacing: 5) {
                    Text("by")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.25))
                    Text("DataGrout")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(.orange.opacity(0.6))
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 8))
                        .foregroundStyle(.orange.opacity(0.4))
                }
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: Certificate

    private var certificateStep: some View {
        VStack(spacing: 20) {
            ZStack {
                Circle()
                    .fill((caTrusted ? Color.green : Color.orange).opacity(0.1))
                    .frame(width: 72, height: 72)
                Image(systemName: caTrusted ? "checkmark.shield.fill" : "lock.shield")
                    .font(.system(size: 30, weight: .light))
                    .foregroundStyle(caTrusted ? .green : .orange)
            }

            VStack(spacing: 8) {
                Text(caTrusted ? "Certificate Installed" : "Install HTTPS Certificate")
                    .font(.system(size: 20, weight: .bold))
                    .foregroundStyle(.white)
                Text(caTrusted
                     ? "Your Lumen CA is already trusted. You're good to go."
                     : "Lumen reads encrypted API traffic. This requires a trusted certificate in your system keychain.")
                    .font(.system(size: 12))
                    .foregroundStyle(.white.opacity(0.5))
                    .multilineTextAlignment(.center)
                    .lineSpacing(3)
            }

            if !caTrusted {
                VStack(spacing: 10) {
                    // One-click trust — uses `security add-trusted-cert` to avoid
                    // the iCloud-keychain default that trips users up in the Keychain dialog.
                    Button(action: trustCertificate) {
                        HStack(spacing: 6) {
                            if caInstallBusy {
                                ProgressView().scaleEffect(0.6).frame(width: 12, height: 12)
                            } else {
                                Image(systemName: "lock.shield")
                                    .font(.system(size: 11))
                            }
                            Text(caInstallBusy ? "Trusting…" : "Trust Certificate")
                                .font(.system(size: 12, weight: .medium))
                        }
                        .foregroundStyle(.orange)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 10)
                        .background(Color.orange.opacity(caInstallBusy ? 0.05 : 0.1))
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                        .overlay(RoundedRectangle(cornerRadius: 8)
                            .stroke(Color.orange.opacity(0.3), lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                    .disabled(caInstallBusy)

                    if let err = caInstallError {
                        Text(err)
                            .font(.system(size: 10))
                            .foregroundStyle(.red.opacity(0.8))
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    } else {
                        Text("macOS will ask for your login password once.")
                            .font(.system(size: 10))
                            .foregroundStyle(.white.opacity(0.3))
                            .multilineTextAlignment(.center)
                    }

                    Button(action: openInKeychain) {
                        Text("Install manually in Keychain Access")
                            .font(.system(size: 9))
                            .foregroundStyle(.white.opacity(0.22))
                            .underline()
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .onAppear {
            checkCATrust()
            // Auto-advance if already trusted
            if caTrusted {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        step = .captureMethod
                    }
                }
            }
        }
    }

    // MARK: Capture Method

    private var captureMethodStep: some View {
        VStack(spacing: 20) {
            VStack(spacing: 8) {
                Text("How do you want to capture traffic?")
                    .font(.system(size: 20, weight: .bold))
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)
                Text("You can change this anytime in Settings.")
                    .font(.system(size: 12))
                    .foregroundStyle(.white.opacity(0.4))
            }

            VStack(spacing: 8) {
                methodCard(
                    method: .proxy,
                    icon: "arrow.triangle.2.circlepath",
                    title: "HTTP Proxy",
                    detail: "Configure each app individually. Precise per-app control.",
                    requirements: ["No root required", "CA cert for HTTPS"],
                    badge: "Recommended"
                )

                methodCard(
                    method: .system,
                    icon: "network",
                    title: "System Proxy",
                    detail: "Route all macOS traffic automatically.",
                    requirements: ["Admin once", "CA cert required"],
                    badge: nil
                )
            }

            if captureMethod == .system {
                Button(action: enableSystemProxy) {
                    HStack(spacing: 6) {
                        if systemProxyBusy {
                            ProgressView().scaleEffect(0.6).frame(width: 12, height: 12)
                        } else {
                            Image(systemName: systemProxyDone ? "checkmark.circle.fill" : "network")
                                .font(.system(size: 11))
                        }
                        Text(systemProxyDone ? "System proxy enabled" : "Enable System Proxy")
                            .font(.system(size: 12, weight: .medium))
                    }
                    .foregroundStyle(systemProxyDone ? .green : .white.opacity(0.8))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 9)
                    .background(Color.white.opacity(0.06))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8)
                        .stroke(Color.white.opacity(0.12), lineWidth: 1))
                }
                .buttonStyle(.plain)
                .disabled(systemProxyBusy || systemProxyDone)
            }

            if captureMethod == .proxy {
                HStack(spacing: 6) {
                    Image(systemName: "info.circle")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.3))
                    Text("Use \(proxyAddress) in your app's HTTP proxy setting, or see Settings for the Claude Code shortcut.")
                        .font(.system(size: 10))
                        .foregroundStyle(.white.opacity(0.35))
                }
                .padding(.horizontal, 4)
            }
        }
    }

    private var proxyAddress: String { "127.0.0.1:\(proxyPort)" }

    private func methodCard(method: CaptureMethod, icon: String, title: String, detail: String, requirements: [String], badge: String?) -> some View {
        let selected = captureMethod == method
        let cardBg = selected ? Color.orange.opacity(0.07) : Color.white.opacity(0.03)
        let border  = selected ? Color.orange.opacity(0.35) : Color.white.opacity(0.08)
        return Button(action: { captureMethod = method }) {
            methodCardContent(selected: selected, icon: icon, title: title, detail: detail, requirements: requirements, badge: badge)
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(cardBg)
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(border, lineWidth: 1))
        }
        .buttonStyle(.plain)
        .focusable(false)
    }

    @ViewBuilder
    private func methodCardContent(selected: Bool, icon: String, title: String, detail: String, requirements: [String], badge: String?) -> some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 16, weight: .light))
                .foregroundStyle(selected ? AnyShapeStyle(Color.orange) : AnyShapeStyle(Color.white.opacity(0.4)))
                .frame(width: 28)

            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(title)
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(selected ? Color.white : Color.white.opacity(0.6))
                    if let badge {
                        Text(badge)
                            .font(.system(size: 8, weight: .semibold))
                            .textCase(.uppercase)
                            .tracking(0.3)
                            .foregroundStyle(Color.orange)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 2)
                            .background(Color.orange.opacity(0.12))
                            .clipShape(RoundedRectangle(cornerRadius: 3))
                    }
                }
                Text(detail)
                    .font(.system(size: 10))
                    .foregroundStyle(Color.white.opacity(0.35))
                HStack(spacing: 4) {
                    ForEach(requirements, id: \.self) { req in
                        Text(req)
                            .font(.system(size: 8, weight: .medium))
                            .foregroundStyle(Color.white.opacity(0.4))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 2)
                            .background(Color.white.opacity(0.06))
                            .clipShape(RoundedRectangle(cornerRadius: 3))
                    }
                }
            }

            Spacer()
            methodCardRadio(selected: selected)
        }
    }

    private func methodCardRadio(selected: Bool) -> some View {
        let strokeColor = selected ? Color.orange : Color.white.opacity(0.2)
        let fillColor   = selected ? Color.orange : Color.clear
        return Circle()
            .stroke(strokeColor, lineWidth: 1.5)
            .background(Circle().fill(fillColor))
            .frame(width: 14, height: 14)
            .overlay(Circle().fill(Color.white).frame(width: 5, height: 5).opacity(selected ? 1 : 0))
    }

    // MARK: Done

    private var doneStep: some View {
        VStack(spacing: 16) {
            ZStack {
                Circle()
                    .fill(Color.green.opacity(0.1))
                    .frame(width: 72, height: 72)
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 40))
                    .foregroundStyle(.green)
                    .shadow(color: .green.opacity(0.4), radius: 12)
            }

            VStack(spacing: 8) {
                Text(startAtDone ? "Lumen is Ready" : "You're all set")
                    .font(.system(size: 22, weight: .bold))
                    .foregroundStyle(.white)
                Text(startAtDone
                     ? "Launch a tool below to start a monitored session."
                     : "Lumen is capturing traffic" + (captureMethod == .proxy ? " via proxy on \(proxyAddress)." : " via system proxy."))
                    .font(.system(size: 13))
                    .foregroundStyle(.white.opacity(0.5))
                    .multilineTextAlignment(.center)
            }

            if !caTrusted {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle")
                        .font(.system(size: 10))
                        .foregroundStyle(.orange)
                    Text("CA certificate not yet trusted — HTTPS traffic won't be decoded. Install it in Settings → Certificate.")
                        .font(.system(size: 10))
                        .foregroundStyle(.orange.opacity(0.7))
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(10)
                .background(Color.orange.opacity(0.06))
                .clipShape(RoundedRectangle(cornerRadius: 8))
            }

            launchShortcuts

            HStack(spacing: 6) {
                Image(systemName: "info.circle")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.25))
                Text("The launchers above handle their own connections — no system proxy or admin password needed. Enable system proxy in Settings only if you want to capture traffic from unconfigured apps.")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.3))
            }
            .padding(.horizontal, 4)

            HStack {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Don't show on startup")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.6))
                    Text("Can be toggled in Settings → About")
                        .font(.system(size: 9))
                        .foregroundStyle(.white.opacity(0.3))
                }
                Spacer()
                Toggle("", isOn: $suppressOnStartup)
                    .toggleStyle(.switch)
                    .scaleEffect(0.7)
                    .focusable(false)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Color.white.opacity(0.03))
            .clipShape(RoundedRectangle(cornerRadius: 8))
        }
    }

    // MARK: - Launch shortcuts (done step)

    private struct AITool: Identifiable {
        let id: String
        let name: String
        let icon: String
        let binaryPaths: [String]
        let interceptionNote: String
        let requiresCA: Bool
        let setupNote: String?
        let envCommand: (String, String) -> String  // (baseURL, caPath) -> shell command
    }

    /// Build tool definitions with no I/O — safe to call from the view body.
    private func buildAIToolDefs() -> [AITool] {
        let home = NSString("~").expandingTildeInPath
        return [
            AITool(
                id: "claude",
                name: "Claude Code",
                icon: "apple.terminal",
                binaryPaths: [
                    "/usr/local/bin/claude",
                    "/opt/homebrew/bin/claude",
                    "\(home)/.local/bin/claude",
                    "\(home)/.npm-global/bin/claude",
                ],
                interceptionNote: "ANTHROPIC_BASE_URL relay",
                requiresCA: false,
                setupNote: nil,
                envCommand: { baseURL, _ in "ANTHROPIC_BASE_URL=\(baseURL) claude" }
            ),
            AITool(
                id: "claude_desktop",
                name: "Claude Desktop",
                icon: "desktopcomputer",
                binaryPaths: ["/Applications/Claude.app"],
                interceptionNote: "System proxy · CA cert required",
                requiresCA: true,
                setupNote: "① Trust Lumen CA (Settings → Certificate)\n② Click Launch — Lumen will enable system proxy and open Claude Desktop",
                envCommand: { _, _ in "open -a Claude" }
            ),
            AITool(
                id: "cursor",
                name: "Cursor",
                icon: "cursorarrow.rays",
                binaryPaths: ["/Applications/Cursor.app/Contents/MacOS/Cursor"],
                interceptionNote: "HTTP/1.1 proxy · CA cert required",
                requiresCA: true,
                setupNote: "① Trust Lumen CA (Settings → Certificate)\n② Cursor Settings → Network → HTTP Compatibility → HTTP/1.1\n③ Click Launch",
                envCommand: { _, ca in
                    "HTTPS_PROXY=http://127.0.0.1:\(proxyPort) NODE_EXTRA_CA_CERTS=\(ca) open -a Cursor"
                }
            ),
            AITool(
                id: "opencode",
                name: "OpenCode",
                icon: "apple.terminal",
                binaryPaths: [
                    "/usr/local/bin/opencode",
                    "/opt/homebrew/bin/opencode",
                    "\(home)/.local/bin/opencode",
                ],
                interceptionNote: "ANTHROPIC_BASE_URL relay",
                requiresCA: false,
                setupNote: nil,
                envCommand: { baseURL, _ in "ANTHROPIC_BASE_URL=\(baseURL) opencode" }
            ),
            AITool(
                id: "hermes",
                name: "Hermes",
                icon: "apple.terminal",
                binaryPaths: ["/usr/local/bin/hermes", "/opt/homebrew/bin/hermes"],
                interceptionNote: "ANTHROPIC_BASE_URL relay",
                requiresCA: false,
                setupNote: nil,
                envCommand: { baseURL, _ in "ANTHROPIC_BASE_URL=\(baseURL) hermes" }
            ),
            AITool(
                id: "pi",
                name: "Pi",
                icon: "apple.terminal",
                binaryPaths: ["/usr/local/bin/pi", "/opt/homebrew/bin/pi"],
                interceptionNote: "ANTHROPIC_BASE_URL relay",
                requiresCA: false,
                setupNote: nil,
                envCommand: { baseURL, _ in "ANTHROPIC_BASE_URL=\(baseURL) pi" }
            ),
        ]
    }

    /// Check whether a tool is installed — runs blocking I/O, call off the main thread.
    private func checkInstalled(_ tool: AITool) -> Bool {
        if tool.binaryPaths.contains(where: { FileManager.default.fileExists(atPath: $0) }) {
            return true
        }
        guard let first = tool.binaryPaths.first else { return false }
        let name = (first as NSString).lastPathComponent
        let proc = Process()
        let pipe = Pipe()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/which")
        proc.arguments = [name]
        proc.standardOutput = pipe
        proc.standardError = Pipe()
        try? proc.run()
        proc.waitUntilExit()
        guard proc.terminationStatus == 0 else { return false }
        let out = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return !out.isEmpty
    }

    private var aiTools: [AITool] { buildAIToolDefs() }

    @State private var launchingTool: String? = nil

    private var displayedToolId: String {
        selectedToolId.isEmpty ? (buildAIToolDefs().first?.id ?? "") : selectedToolId
    }

    private var launchBaseURL: String {
        "http://127.0.0.1:\(proxyPort)/anthropic"
    }
    private var launchCAPath: String {
        apiClient.caInfo?.path ?? NSString("~/.lumen/ca.pem").expandingTildeInPath
    }

    // First 4 tools shown as pills; the rest collapse into "More"
    private var primaryTools: [AITool] { Array(buildAIToolDefs().prefix(4)) }
    private var moreTools: [AITool]    { Array(buildAIToolDefs().dropFirst(4)) }

    private var launchShortcuts: some View {
        let primary = primaryTools
        let more    = moreTools
        let visibleTools = showMoreTools ? more : primary
        let displayed = buildAIToolDefs().first(where: { $0.id == displayedToolId })

        return VStack(alignment: .leading, spacing: 8) {
            Text("Launch with Lumen")
                .font(.system(size: 9, weight: .medium))
                .foregroundStyle(.white.opacity(0.35))
                .textCase(.uppercase)
                .tracking(0.5)

            HStack(spacing: 6) {
                ForEach(visibleTools) { tool in
                    toolPill(tool)
                }
                if !more.isEmpty {
                    Button(action: {
                        showMoreTools.toggle()
                        if !showMoreTools && more.contains(where: { $0.id == displayedToolId }) {
                            selectedToolId = ""
                        }
                    }) {
                        Text(showMoreTools ? "Back" : "More")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(showMoreTools ? .black : Color.white.opacity(0.45))
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(showMoreTools ? Color.white.opacity(0.5) : Color.white.opacity(0.04))
                            .clipShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .focusable(false)
                }
            }

            if let tool = displayed {
                toolDetailCard(tool)
            }
        }
    }

    private func toolPill(_ tool: AITool) -> some View {
        let isSelected = displayedToolId == tool.id
        let isInstalled = installedTools.contains(tool.id)
        return Button(action: { selectedToolId = tool.id; showMoreTools = primaryTools.contains(where: { $0.id == tool.id }) ? false : showMoreTools }) {
            Text(tool.name)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(
                    isSelected ? .black
                    : isInstalled ? Color.white.opacity(0.7)
                    : Color.white.opacity(0.3)
                )
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(
                    isSelected ? Color.orange
                    : Color.white.opacity(isInstalled ? 0.06 : 0.03)
                )
                .clipShape(Capsule())
        }
        .buttonStyle(.plain)
        .focusable(false)
    }

    private func toolDetailCard(_ tool: AITool) -> some View {
        let isInstalled = installedTools.contains(tool.id)
        let caBlocked = tool.requiresCA && !caTrusted
        let canLaunch = isInstalled && !caBlocked && launchingTool == nil

        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Text(tool.name)
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(isInstalled ? Color.white.opacity(0.9) : Color.white.opacity(0.4))
                        if !isInstalled {
                            Text("not found")
                                .font(.system(size: 8))
                                .foregroundStyle(.white.opacity(0.3))
                                .padding(.horizontal, 5)
                                .padding(.vertical, 2)
                                .background(Color.white.opacity(0.05))
                                .clipShape(RoundedRectangle(cornerRadius: 3))
                        }
                    }
                    noteLabel(tool)
                }
                Spacer()
                Button(action: { launchTool(tool, baseURL: launchBaseURL, caPath: launchCAPath) }) {
                    HStack(spacing: 4) {
                        if launchingTool == tool.id {
                            ProgressView().scaleEffect(0.5).frame(width: 10, height: 10)
                        } else {
                            Image(systemName: caBlocked ? "lock.fill" : "play.fill").font(.system(size: 8))
                        }
                        Text(caBlocked ? "Cert needed" : "Launch").font(.system(size: 10, weight: .semibold))
                    }
                    .foregroundStyle(canLaunch ? .black : Color.white.opacity(0.2))
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(canLaunch ? Color.orange : Color.white.opacity(0.04))
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                }
                .buttonStyle(.plain)
                .disabled(!canLaunch)
            }

            if let note = tool.setupNote {
                Divider()
                    .background(Color.white.opacity(0.08))
                    .padding(.vertical, 8)
                Text(note)
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.45))
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 10)
        .background(Color.white.opacity(0.03))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    @ViewBuilder
    private func noteLabel(_ tool: AITool) -> some View {
        if tool.requiresCA {
            HStack(spacing: 4) {
                Text(tool.interceptionNote)
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.35))
                if !caTrusted {
                    Text("· CA cert missing")
                        .font(.system(size: 9, weight: .medium))
                        .foregroundStyle(Color.orange.opacity(0.8))
                } else {
                    Text("· cert trusted")
                        .font(.system(size: 9))
                        .foregroundStyle(Color.green.opacity(0.7))
                }
            }
        } else {
            HStack(spacing: 4) {
                Text(tool.interceptionNote)
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.35))
                Text("· no cert needed")
                    .font(.system(size: 9))
                    .foregroundStyle(Color.green.opacity(0.7))
            }
        }
    }

    private func launchTool(_ tool: AITool, baseURL: String, caPath: String) {
        launchingTool = tool.id
        let cmd = tool.envCommand(baseURL, caPath)

        DispatchQueue.global(qos: .userInitiated).async {
            if tool.id == "claude_desktop" {
                // Claude Desktop doesn't inherit env vars from open -a, so enable system proxy first
                let port = self.proxyPort
                let iface = SystemProxy.activeInterface()
                Task { _ = await SystemProxy.enable(port: port, interface: iface) }
                let proc = Process()
                proc.executableURL = URL(fileURLWithPath: "/usr/bin/open")
                proc.arguments = ["-a", "Claude"]
                proc.standardOutput = FileHandle.nullDevice
                proc.standardError  = FileHandle.nullDevice
                try? proc.run()
            } else if tool.id == "cursor" {
                let proc = Process()
                proc.executableURL = URL(fileURLWithPath: "/Applications/Cursor.app/Contents/MacOS/Cursor")
                proc.arguments = ["--proxy-server=http://127.0.0.1:\(proxyPort)"]
                var env = ProcessInfo.processInfo.environment
                env["HTTPS_PROXY"] = "http://127.0.0.1:\(proxyPort)"
                env["HTTP_PROXY"]  = "http://127.0.0.1:\(proxyPort)"
                env["NODE_EXTRA_CA_CERTS"] = caPath
                proc.environment = env
                proc.standardOutput = FileHandle.nullDevice
                proc.standardError  = FileHandle.nullDevice
                try? proc.run()
            } else {
                let echo = "echo '✓ Lumen active — tracking \(tool.name). Run: \(tool.id)'"
                let script = """
                tell application "Terminal"
                    activate
                    do script "\(cmd.replacingOccurrences(of: "\"", with: "\\\"")) && \(echo)"
                end tell
                """
                let proc = Process()
                proc.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
                proc.arguments = ["-e", script]
                proc.standardOutput = FileHandle.nullDevice
                proc.standardError  = FileHandle.nullDevice
                try? proc.run()
                proc.waitUntilExit()
            }

            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                launchingTool = nil
            }
        }
    }

    // MARK: - Navigation

    private var navigationRow: some View {
        HStack {
            if step != .welcome {
                Button(action: goBack) {
                    Text("Back")
                        .font(.system(size: 12))
                        .foregroundStyle(.white.opacity(0.4))
                }
                .buttonStyle(.plain)
                .focusable(false)
            }

            Spacer()

            Button(action: goNext) {
                Text(nextLabel)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.black)
                    .padding(.horizontal, 28)
                    .padding(.vertical, 10)
                    .background(Color.orange)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }
            .buttonStyle(.plain)
            .focusable(false)
        }
    }

    private var nextLabel: String {
        switch step {
        case .welcome:       return "Get Started"
        case .certificate:   return caTrusted ? "Continue" : "Skip for now"
        case .captureMethod: return "Continue"
        case .done:          return "Start Monitoring"
        }
    }

    private func goNext() {
        if step == .done { onComplete(); return }

        var next = WizardStep(rawValue: step.rawValue + 1) ?? .done
        // Skip cert step if already trusted
        if next == .certificate && caTrusted { next = .captureMethod }

        withAnimation(.easeInOut(duration: 0.2)) { step = next }
    }

    private func goBack() {
        var prev = WizardStep(rawValue: step.rawValue - 1) ?? .welcome
        // Skip cert step going back if already trusted
        if prev == .certificate && caTrusted { prev = .welcome }
        withAnimation(.easeInOut(duration: 0.2)) { step = prev }
    }

    // MARK: - Helpers

    private func trustCertificate() {
        guard let path = apiClient.caInfo?.path else { return }
        caInstallBusy = true
        caInstallError = nil

        DispatchQueue.global(qos: .userInitiated).async {
            // `security add-trusted-cert` imports the cert to the login keychain
            // and sets user-domain trust — no admin required, no iCloud confusion.
            let loginKeychain = NSString("~/Library/Keychains/login.keychain-db")
                .expandingTildeInPath
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/security")
            proc.arguments = ["add-trusted-cert", "-r", "trustRoot",
                              "-k", loginKeychain, path]
            proc.standardOutput = Pipe()
            let errPipe = Pipe()
            proc.standardError = errPipe

            do {
                try proc.run()
                proc.waitUntilExit()
            } catch {
                DispatchQueue.main.async {
                    caInstallBusy = false
                    caInstallError = error.localizedDescription
                }
                return
            }

            let errData = errPipe.fileHandleForReading.readDataToEndOfFile()
            let errStr  = String(data: errData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""

            DispatchQueue.main.async {
                caInstallBusy = false
                if proc.terminationStatus == 0 {
                    caTrusted = true
                    caInstallError = nil
                    // Auto-advance
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                        withAnimation(.easeInOut(duration: 0.2)) { step = .captureMethod }
                    }
                } else if errStr.localizedCaseInsensitiveContains("cancelled") {
                    caInstallError = "Cancelled — enter your login password to trust the certificate."
                } else {
                    caInstallError = errStr.isEmpty
                        ? "Trust failed (code \(proc.terminationStatus)). Try the manual option below."
                        : errStr
                }
            }
        }
    }

    private func openInKeychain() {
        guard let path = apiClient.caInfo?.path else { return }
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        proc.arguments = [path]
        try? proc.run()
        // Note shown after manual install: user must pick "login" not "iCloud"
        // and then set Always Trust. Poll a few times to catch when they do.
        for delay in [3.0, 6.0, 12.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { checkCATrust() }
        }
    }

    private func checkCATrust() {
        DispatchQueue.global(qos: .utility).async {
            // dump-trust-settings checks the actual trust database, not just cert
            // existence — avoids false-positives from iCloud keychain entries that
            // were imported without trust settings.
            let proc = Process()
            let pipe = Pipe()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/security")
            proc.arguments = ["dump-trust-settings"]
            proc.standardOutput = pipe
            proc.standardError  = Pipe()
            try? proc.run()
            proc.waitUntilExit()
            let out = String(data: pipe.fileHandleForReading.readDataToEndOfFile(),
                             encoding: .utf8) ?? ""
            let trusted = out.localizedCaseInsensitiveContains("Lumen Local CA")
            DispatchQueue.main.async { caTrusted = trusted }
        }
    }

    private func enableSystemProxy() {
        systemProxyBusy = true
        Task {
            let ok = await SystemProxy.enable(port: apiClient.proxyConfig.port)
            await MainActor.run {
                systemProxyBusy = false
                systemProxyDone = ok
            }
        }
    }
}
