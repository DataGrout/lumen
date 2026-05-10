import SwiftUI
import AppKit

struct SettingsView: View {
    let apiClient: APIClient
    let daemonManager: DaemonManager
    @State private var showAddRoute = false
    @State private var newRoutePrefix = ""
    @State private var newRouteUpstream = ""
    @State private var systemProxyEnabled = false
    @State private var activeInterface = "Wi-Fi"
    @State private var caTrusted = false
    @State private var caInstallBusy = false
    @State private var caInstallError: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            setupSection
            routesSection
            certificateSection
            proxySection
            transparentSection
            dgSection
            aboutSection
        }
    }

    private func stepRow(number: Int, text: String) -> some View {
        HStack(alignment: .top, spacing: 6) {
            Text("\(number).")
                .font(.system(size: 10, weight: .bold, design: .monospaced))
                .foregroundStyle(.orange.opacity(0.7))
                .frame(width: 16, alignment: .trailing)
            Text(text)
                .font(.system(size: 10))
                .foregroundStyle(.white.opacity(0.6))
        }
    }

    private func copyableCode(_ text: String) -> some View {
        HStack(spacing: 4) {
            Text(text)
                .font(.system(size: 9, design: .monospaced))
                .foregroundStyle(.orange.opacity(0.8))
                .lineLimit(1)
            Spacer()
            Button(action: {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(text, forType: .string)
            }) {
                Image(systemName: "doc.on.doc")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.4))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(Color.white.opacity(0.05))
        .clipShape(RoundedRectangle(cornerRadius: 4))
        .padding(.leading, 22)
    }

    // MARK: - Relay Setup

    private var setupSection: some View {
        settingsGroup("Quick Setup") {
            VStack(alignment: .leading, spacing: 6) {
                Text("Point your LLM client at Lumen to start monitoring.")
                    .font(.system(size: 10))
                    .foregroundStyle(.white.opacity(0.5))

                ForEach(apiClient.routes) { route in
                    let relayURL = "http://localhost:\(apiClient.proxyConfig.port)\(route.prefix)"
                    setupRow(
                        provider: providerName(for: route.prefix),
                        url: relayURL
                    )
                }

                Text("Set the URL above as your API base URL (e.g., Cursor Settings > Models > Override OpenAI Base URL).")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.3))
                    .padding(.top, 2)
            }
        }
    }

    private func setupRow(provider: String, url: String) -> some View {
        HStack(spacing: 6) {
            Text(provider)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(.orange.opacity(0.8))
                .frame(width: 60, alignment: .leading)
            Text(url)
                .font(.system(size: 9, design: .monospaced))
                .foregroundStyle(.white.opacity(0.6))
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer()
            Button(action: {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(url, forType: .string)
            }) {
                Image(systemName: "doc.on.doc")
                    .font(.system(size: 9))
                    .foregroundStyle(.white.opacity(0.4))
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.white.opacity(0.03))
        .clipShape(RoundedRectangle(cornerRadius: 5))
    }

    // MARK: - Routes

    private var routesSection: some View {
        settingsGroup("Relay Routes") {
            ForEach(apiClient.routes) { route in
                HStack {
                    Text(route.prefix)
                        .font(.system(size: 10, weight: .medium, design: .monospaced))
                        .foregroundStyle(.white.opacity(0.7))
                    Image(systemName: "arrow.right")
                        .font(.system(size: 8))
                        .foregroundStyle(.white.opacity(0.2))
                    Text(route.upstream)
                        .font(.system(size: 9, design: .monospaced))
                        .foregroundStyle(.white.opacity(0.4))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer()
                    Button(action: { Task { await apiClient.removeRoute(route.prefix) } }) {
                        Image(systemName: "xmark")
                            .font(.system(size: 8))
                            .foregroundStyle(.white.opacity(0.3))
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Color.white.opacity(0.03))
                .clipShape(RoundedRectangle(cornerRadius: 5))
            }

            if showAddRoute {
                VStack(spacing: 4) {
                    HStack(spacing: 4) {
                        TextField("/prefix", text: $newRoutePrefix)
                            .textFieldStyle(.plain)
                            .font(.system(size: 10, design: .monospaced))
                            .padding(4)
                            .background(Color.white.opacity(0.06))
                            .clipShape(RoundedRectangle(cornerRadius: 4))
                            .frame(width: 80)
                        TextField("https://upstream.host", text: $newRouteUpstream)
                            .textFieldStyle(.plain)
                            .font(.system(size: 10, design: .monospaced))
                            .padding(4)
                            .background(Color.white.opacity(0.06))
                            .clipShape(RoundedRectangle(cornerRadius: 4))
                        Button("Add") {
                            guard !newRoutePrefix.isEmpty, !newRouteUpstream.isEmpty else { return }
                            Task { await apiClient.addRoute(prefix: newRoutePrefix, upstream: newRouteUpstream) }
                            newRoutePrefix = ""
                            newRouteUpstream = ""
                            showAddRoute = false
                        }
                        .font(.system(size: 9, weight: .medium))
                        .buttonStyle(.plain)
                        .foregroundStyle(.orange)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Color.white.opacity(0.03))
                .clipShape(RoundedRectangle(cornerRadius: 5))
            }

            Button(action: { showAddRoute.toggle() }) {
                HStack(spacing: 4) {
                    Image(systemName: showAddRoute ? "minus" : "plus")
                        .font(.system(size: 8))
                    Text(showAddRoute ? "Cancel" : "Add Route")
                        .font(.system(size: 9))
                }
                .foregroundStyle(.white.opacity(0.4))
            }
            .buttonStyle(.plain)
        }
    }

    // MARK: - Certificate

    private var certificateSection: some View {
        settingsGroup("TLS Certificate (for HTTPS Proxy Mode)") {
            if let info = apiClient.caInfo {
                HStack {
                    Text("Status")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.6))
                    Spacer()
                    Text(caTrusted ? "Trusted" : "Not Trusted")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(caTrusted ? .green : .orange)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(Color.white.opacity(0.03))
                .clipShape(RoundedRectangle(cornerRadius: 5))

                if let path = info.path {
                    HStack {
                        Text("CA File")
                            .font(.system(size: 11))
                            .foregroundStyle(.white.opacity(0.6))
                        Spacer()
                        Text(path)
                            .font(.system(size: 8, design: .monospaced))
                            .foregroundStyle(.white.opacity(0.3))
                            .lineLimit(1)
                            .truncationMode(.head)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background(Color.white.opacity(0.03))
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                }
            }

            if !caTrusted {
                Button(action: trustCA) {
                    HStack(spacing: 4) {
                        if caInstallBusy {
                            ProgressView().scaleEffect(0.5).frame(width: 10, height: 10)
                        } else {
                            Image(systemName: "lock.shield").font(.system(size: 9))
                        }
                        Text(caInstallBusy ? "Trusting…" : "Trust Certificate")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .foregroundStyle(.orange.opacity(0.9))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 7)
                    .background(Color.orange.opacity(0.08))
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                    .overlay(RoundedRectangle(cornerRadius: 5)
                        .stroke(Color.orange.opacity(0.2), lineWidth: 1))
                }
                .buttonStyle(.plain)
                .disabled(caInstallBusy)
            }

            if let err = caInstallError {
                Text(err)
                    .font(.system(size: 9))
                    .foregroundStyle(.red.opacity(0.8))
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 8) {
                Button(action: openInKeychain) {
                    HStack(spacing: 4) {
                        Image(systemName: "lock.shield").font(.system(size: 9))
                        Text("Open in Keychain").font(.system(size: 10, weight: .medium))
                    }
                    .foregroundStyle(.white.opacity(0.4))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 7)
                    .background(Color.white.opacity(0.04))
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                }
                .buttonStyle(.plain)

                Button(action: revealCA) {
                    HStack(spacing: 4) {
                        Image(systemName: "folder").font(.system(size: 9))
                        Text("Reveal").font(.system(size: 10, weight: .medium))
                    }
                    .foregroundStyle(.white.opacity(0.5))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 7)
                    .background(Color.white.opacity(0.04))
                    .clipShape(RoundedRectangle(cornerRadius: 5))
                }
                .buttonStyle(.plain)
            }

            Text(caTrusted
                 ? "Certificate is trusted. HTTPS proxy mode is active."
                 : "macOS will ask for your login password once to set trust.")
                .font(.system(size: 9))
                .foregroundStyle(.white.opacity(0.3))
        }
        .onAppear { checkCATrust() }
    }

    private func trustCA() {
        guard let path = apiClient.caInfo?.path else { return }
        caInstallBusy = true
        caInstallError = nil

        DispatchQueue.global(qos: .userInitiated).async {
            let loginKeychain = NSString("~/Library/Keychains/login.keychain-db")
                .expandingTildeInPath
            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/security")
            proc.arguments = ["add-trusted-cert", "-r", "trustRoot",
                              "-k", loginKeychain, path]
            proc.standardOutput = Pipe()
            let errPipe = Pipe()
            proc.standardError = errPipe
            do { try proc.run(); proc.waitUntilExit() } catch {
                DispatchQueue.main.async { caInstallBusy = false
                    caInstallError = error.localizedDescription }
                return
            }
            let errStr = String(data: errPipe.fileHandleForReading.readDataToEndOfFile(),
                                encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            DispatchQueue.main.async {
                caInstallBusy = false
                if proc.terminationStatus == 0 {
                    caTrusted = true; caInstallError = nil
                } else if errStr.localizedCaseInsensitiveContains("cancelled") {
                    caInstallError = "Cancelled — enter your login password to trust."
                } else {
                    caInstallError = errStr.isEmpty
                        ? "Trust failed (code \(proc.terminationStatus))."
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
    }

    private func checkCATrust() {
        DispatchQueue.global(qos: .utility).async {
            // Check the actual trust settings database, not just cert existence.
            // find-certificate gives false positives when the cert is in iCloud
            // keychain with no trust settings.
            let proc = Process()
            let pipe = Pipe()
            proc.executableURL = URL(fileURLWithPath: "/usr/bin/security")
            proc.arguments = ["dump-trust-settings"]
            proc.standardOutput = pipe
            proc.standardError = Pipe()
            try? proc.run()
            proc.waitUntilExit()
            let out = String(data: pipe.fileHandleForReading.readDataToEndOfFile(),
                             encoding: .utf8) ?? ""
            let trusted = out.localizedCaseInsensitiveContains("Lumen Local CA")
            DispatchQueue.main.async { caTrusted = trusted }
        }
    }

    private func revealCA() {
        guard let path = apiClient.caInfo?.path else { return }
        NSWorkspace.shared.selectFile(path, inFileViewerRootedAtPath: "")
    }

    // MARK: - Standard sections

    private func refreshProxyState() {
        DispatchQueue.global(qos: .utility).async {
            let iface = SystemProxy.activeInterface()
            let enabled = SystemProxy.isEnabled(interface: iface)
            DispatchQueue.main.async {
                activeInterface = iface
                systemProxyEnabled = enabled
            }
        }
    }

    private var proxySection: some View {
        settingsGroup("Proxy") {
            settingRow("Port", value: "\(apiClient.proxyConfig.port)")
            settingRow("Status", value: apiClient.proxyConfig.running ? "Running" : "Stopped",
                      highlight: apiClient.proxyConfig.running)
            settingRow("Interface", value: activeInterface)
            toggleRow("System Proxy", isOn: systemProxyEnabled) { val in
                Task {
                    if val {
                        let ok = await SystemProxy.enable(port: apiClient.proxyConfig.port, interface: activeInterface)
                        await MainActor.run {
                            systemProxyEnabled = ok
                            if ok {
                                UserDefaults.standard.set(true, forKey: "lumen.autoEnableProxy")
                                UserDefaults.standard.set(apiClient.proxyConfig.port, forKey: "lumen.proxyPort")
                            }
                        }
                    } else {
                        let ok = await SystemProxy.disable(interface: activeInterface)
                        await MainActor.run {
                            systemProxyEnabled = !ok
                            if ok {
                                UserDefaults.standard.set(false, forKey: "lumen.autoEnableProxy")
                            }
                        }
                    }
                }
            }
            if systemProxyEnabled {
                Text("System traffic is routed through Lumen. Monitored HTTPS hosts will be decrypted (requires trusted CA).")
                    .font(.system(size: 9))
                    .foregroundStyle(.orange.opacity(0.5))
            }
        }
        .onAppear { refreshProxyState() }
        .onChange(of: apiClient.connected) { refreshProxyState() }
    }

    // MARK: - Transparent Capture

    @State private var transparentEnabled = false

    private var transparentSection: some View {
        settingsGroup("Transparent Capture (Advanced)") {
            VStack(alignment: .leading, spacing: 6) {
                Text("Capture ALL outbound LLM traffic without configuring individual apps. Uses macOS pf firewall to redirect HTTPS connections to monitored API hosts through Lumen's transparent proxy.")
                    .font(.system(size: 10))
                    .foregroundStyle(.white.opacity(0.5))

                HStack {
                    Text("Status")
                        .font(.system(size: 11))
                        .foregroundStyle(.white.opacity(0.6))
                    Spacer()
                    Text(transparentEnabled ? "Active" : "Inactive")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(transparentEnabled ? .green : .white.opacity(0.4))
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(Color.white.opacity(0.03))
                .clipShape(RoundedRectangle(cornerRadius: 5))

                if !transparentEnabled {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Setup requires one-time admin privileges:")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.white.opacity(0.5))

                        stepRow(number: 1, text: "Trust the Lumen CA in Keychain (see above)")
                        stepRow(number: 2, text: "Create a system user for loop avoidance (one-time)")
                        stepRow(number: 3, text: "Run lumen-core with --transparent flag")
                        stepRow(number: 4, text: "Enable pf redirect rules (requires sudo)")
                    }

                    let enableCmd = "sudo scripts/pf_setup.sh --local"
                    copyableCode(enableCmd)

                    Text("Transparent capture intercepts only HTTPS traffic to known LLM API endpoints. All other traffic passes through unmodified. The interception endpoints are visible in the Traffic tab.")
                        .font(.system(size: 9))
                        .foregroundStyle(.white.opacity(0.3))
                        .padding(.top, 2)
                } else {
                    Button(action: disableTransparent) {
                        HStack(spacing: 6) {
                            Image(systemName: "xmark.circle")
                                .font(.system(size: 10))
                            Text("Disable Transparent Capture")
                                .font(.system(size: 11, weight: .medium))
                        }
                        .foregroundStyle(.red.opacity(0.8))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .background(Color.red.opacity(0.08))
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(Color.red.opacity(0.2), lineWidth: 1)
                        )
                    }
                    .buttonStyle(.plain)

                    Text("Removes pf redirect rules. Requires admin privileges.")
                        .font(.system(size: 9))
                        .foregroundStyle(.white.opacity(0.3))
                }
            }
        }
        .onAppear { checkTransparentStatus() }
    }

    private func checkTransparentStatus() {
        Task {
            guard let url = URL(string: "http://127.0.0.1:\(apiClient.proxyConfig.port + 1)/transparent/config") else { return }
            do {
                let (data, _) = try await URLSession.shared.data(from: url)
                if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let enabled = json["enabled"] as? Bool {
                    await MainActor.run { transparentEnabled = enabled }
                }
            } catch {
                // API not reachable
            }
        }
    }

    private func disableTransparent() {
        let script = "do shell script \"pfctl -a com.datagrout.lumen -F all 2>/dev/null\" with administrator privileges"
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        proc.arguments = ["-e", script]
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        try? proc.run()
        proc.waitUntilExit()
        transparentEnabled = false
    }

    // MARK: - DataGrout

    @State private var dgServerURL = ""
    @State private var dgConnecting = false
    @State private var dgAuthURL: String? = nil
    @State private var dgConnectError: String? = nil
    @State private var dgPollTimer: Timer? = nil

    private var dgConnected: Bool {
        apiClient.dgStatus?.connected == true
    }

    private var dgSection: some View {
        settingsGroup("DataGrout") {
            HStack {
                Text("Sync Status")
                    .font(.system(size: 11))
                    .foregroundStyle(.white.opacity(0.6))
                Spacer()
                HStack(spacing: 4) {
                    Circle()
                        .fill(dgConnected ? Color.green : Color.white.opacity(0.2))
                        .frame(width: 6, height: 6)
                    Text(dgConnected ? "Connected" : "Not connected")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(dgConnected ? .green : .white.opacity(0.4))
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Color.white.opacity(0.03))
            .clipShape(RoundedRectangle(cornerRadius: 5))

            if dgConnected {
                if let subId = apiClient.dgStatus?.subId {
                    settingRow("Device ID", value: subId)
                }
                if let url = apiClient.dgStatus?.serverUrl {
                    settingRow("Server", value: url)
                }
                Button(action: {
                    Task { await apiClient.disconnectDG() }
                }) {
                    Text("Disconnect")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(.red.opacity(0.8))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 6)
                        .background(Color.red.opacity(0.08))
                        .clipShape(RoundedRectangle(cornerRadius: 5))
                        .overlay(RoundedRectangle(cornerRadius: 5).stroke(Color.red.opacity(0.2), lineWidth: 1))
                }
                .buttonStyle(.plain)
                .focusable(false)
            } else {
                VStack(spacing: 6) {
                    TextField("https://…datagrout.ai/servers/UUID/mcp", text: $dgServerURL)
                        .textFieldStyle(.plain)
                        .font(.system(size: 10, design: .monospaced))
                        .padding(.horizontal, 8)
                        .padding(.vertical, 6)
                        .background(Color.white.opacity(0.06))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                        .onAppear {
                            dgServerURL = apiClient.dgConfig.serverUrl ?? ""
                        }

                    if let authURL = dgAuthURL {
                        HStack(spacing: 6) {
                            Image(systemName: "globe")
                                .font(.system(size: 9))
                                .foregroundStyle(.orange.opacity(0.7))
                            Text("Authorize in browser, then wait…")
                                .font(.system(size: 10))
                                .foregroundStyle(.white.opacity(0.5))
                            Spacer()
                            Button(action: {
                                if let url = URL(string: authURL) {
                                    NSWorkspace.shared.open(url)
                                }
                            }) {
                                Text("Reopen")
                                    .font(.system(size: 9))
                                    .foregroundStyle(.orange.opacity(0.7))
                            }
                            .buttonStyle(.plain)
                        }
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .background(Color.orange.opacity(0.06))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                        .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.orange.opacity(0.2), lineWidth: 1))
                    }

                    if let err = dgConnectError {
                        Text(err)
                            .font(.system(size: 9))
                            .foregroundStyle(.red.opacity(0.8))
                    }

                    Button(action: startDCRConnect) {
                        HStack(spacing: 6) {
                            if dgConnecting {
                                ProgressView()
                                    .scaleEffect(0.5)
                                    .frame(width: 12, height: 12)
                            } else {
                                Image(systemName: "arrow.up.right.square")
                                    .font(.system(size: 10))
                            }
                            Text(dgConnecting ? (dgAuthURL == nil ? "Registering…" : "Waiting for browser…") : "Connect with Browser")
                                .font(.system(size: 11, weight: .medium))
                        }
                        .foregroundStyle(.black)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 8)
                        .background(dgServerURL.isEmpty ? Color.orange.opacity(0.4) : Color.orange)
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                    .buttonStyle(.plain)
                    .disabled(dgConnecting || dgServerURL.isEmpty)

                    Button(action: {
                        if let url = URL(string: "https://app.datagrout.ai") {
                            NSWorkspace.shared.open(url)
                        }
                    }) {
                        Text("Don't have an account? Create one free →")
                            .font(.system(size: 10))
                            .foregroundStyle(.white.opacity(0.35))
                    }
                    .buttonStyle(.plain)

                    if dgConnecting && dgAuthURL == nil {
                        Text("Registering client with server…")
                            .font(.system(size: 9))
                            .foregroundStyle(.white.opacity(0.3))
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(Color.white.opacity(0.03))
                .clipShape(RoundedRectangle(cornerRadius: 5))
            }

            Divider()
                .background(.white.opacity(0.08))
                .padding(.vertical, 2)

            toggleRow("Hide DG Tools", isOn: Binding(
                get: { apiClient.dgConfig.toolsHidden },
                set: { val in
                    apiClient.dgConfig.toolsHidden = val
                    let config = apiClient.dgConfig
                    Task { await apiClient.updateDGConfig(config) }
                }
            ))
            toggleRow("Intelligent Interface", isOn: Binding(
                get: { apiClient.dgConfig.intelligentInterface },
                set: { val in
                    apiClient.dgConfig.intelligentInterface = val
                    let config = apiClient.dgConfig
                    Task { await apiClient.updateDGConfig(config) }
                }
            ))
        }
    }

    private func startDCRConnect() {
        dgConnecting = true
        dgConnectError = nil
        dgAuthURL = nil
        let serverUrl = dgServerURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let deviceName = Host.current().localizedName ?? ProcessInfo.processInfo.hostName

        Task {
            guard let result = await apiClient.startDCRFlow(serverUrl: serverUrl, deviceName: deviceName) else {
                await MainActor.run {
                    dgConnecting = false
                    dgConnectError = "DCR registration failed — check the server URL."
                }
                return
            }

            await MainActor.run {
                dgAuthURL = result.authUrl
                if let url = URL(string: result.authUrl) {
                    NSWorkspace.shared.open(url)
                }
                startDCRPoll()
            }
        }
    }

    private func startDCRPoll() {
        dgPollTimer?.invalidate()
        dgPollTimer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { _ in
            Task { await checkDCRStatus() }
        }
    }

    private func checkDCRStatus() async {
        guard let json = await apiClient.getDCRStatus(),
              let status = json["status"] as? String else { return }

        await MainActor.run {
            switch status {
            case "complete":
                dgPollTimer?.invalidate()
                dgPollTimer = nil
                dgConnecting = false
                dgAuthURL = nil
                Task { await apiClient.fetchDGStatus() }
            case "failed":
                dgPollTimer?.invalidate()
                dgPollTimer = nil
                dgConnecting = false
                dgAuthURL = nil
                dgConnectError = json["error"] as? String ?? "Authorization failed."
            default:
                break
            }
        }
    }

    @AppStorage("lumen.suppressLauncher") private var suppressLauncher = false

    private var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown"
    }

    private var aboutSection: some View {
        settingsGroup("About") {
            settingRow("Version", value: appVersion)
            settingRow("License", value: "MIT")
            settingRow("Daemon", value: daemonManager.isRunning ? "Running" : "Stopped",
                      highlight: daemonManager.isRunning)
            toggleRow("Show Launcher on Startup", isOn: !suppressLauncher) { val in
                suppressLauncher = !val
            }
        }
    }

    // MARK: - Helpers

    private func providerName(for prefix: String) -> String {
        switch prefix {
        case "/openai": return "OpenAI"
        case "/anthropic": return "Anthropic"
        case "/google": return "Google"
        default: return String(prefix.dropFirst()).capitalized
        }
    }

    @ViewBuilder
    private func settingsGroup<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(.system(size: 9, weight: .medium))
                .foregroundStyle(.white.opacity(0.35))
                .textCase(.uppercase)
                .tracking(0.5)
                .padding(.bottom, 2)
            content()
        }
    }

    private func settingRow(_ label: String, value: String, highlight: Bool = false) -> some View {
        HStack {
            Text(label)
                .font(.system(size: 11))
                .foregroundStyle(.white.opacity(0.6))
            Spacer()
            Text(value)
                .font(.system(size: 10))
                .foregroundStyle(highlight ? .green : .white.opacity(0.4))
                .monospacedDigit()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background(Color.white.opacity(0.03))
        .clipShape(RoundedRectangle(cornerRadius: 5))
    }

    private func toggleRow(_ label: String, isOn: Bool, action: @escaping (Bool) -> Void) -> some View {
        toggleRow(label, isOn: Binding(get: { isOn }, set: { action($0) }))
    }

    private func toggleRow(_ label: String, isOn: Binding<Bool>) -> some View {
        HStack {
            Text(label)
                .font(.system(size: 11))
                .foregroundStyle(.white.opacity(0.6))
            Spacer()
            Toggle("", isOn: isOn)
                .toggleStyle(.switch)
                .tint(.orange)
                .scaleEffect(0.7)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.white.opacity(0.03))
        .clipShape(RoundedRectangle(cornerRadius: 5))
    }
}
