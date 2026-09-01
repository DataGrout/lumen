import Foundation

struct AggregateStats: Codable {
    var totalInputTokens: Int = 0
    var totalOutputTokens: Int = 0
    var totalTokens: Int = 0
    var totalCost: Double = 0
    var totalCacheSavings: Double = 0
    var eventCount: Int = 0
    var sessionInputTokens: Int = 0
    var sessionOutputTokens: Int = 0
    var sessionCacheReadTokens: Int = 0
    var sessionCacheCreationTokens: Int = 0
    var sessionCost: Double = 0
    var sessionCacheSavings: Double = 0
    var tokensPerMinute: Double = 0
    var costPerMinute: Double = 0
    var topModel: String? = nil
    var topProvider: String? = nil
    var currentLap: Int = 1

    enum CodingKeys: String, CodingKey {
        case totalInputTokens = "total_input_tokens"
        case totalOutputTokens = "total_output_tokens"
        case totalTokens = "total_tokens"
        case totalCost = "total_cost"
        case totalCacheSavings = "total_cache_savings"
        case eventCount = "event_count"
        case sessionInputTokens = "session_input_tokens"
        case sessionOutputTokens = "session_output_tokens"
        case sessionCacheReadTokens = "session_cache_read_tokens"
        case sessionCacheCreationTokens = "session_cache_creation_tokens"
        case sessionCost = "session_cost"
        case sessionCacheSavings = "session_cache_savings"
        case tokensPerMinute = "tokens_per_minute"
        case costPerMinute = "cost_per_minute"
        case topModel = "top_model"
        case topProvider = "top_provider"
        case currentLap = "current_lap"
    }
}

struct TokenUsage: Codable {
    var inputTokens: Int
    var outputTokens: Int
    var totalTokens: Int
    var cacheReadTokens: Int?
    var cacheCreationTokens: Int?

    enum CodingKeys: String, CodingKey {
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case totalTokens = "total_tokens"
        case cacheReadTokens = "cache_read_tokens"
        case cacheCreationTokens = "cache_creation_tokens"
    }
}

struct CostBreakdown: Codable {
    var inputCost: Double
    var outputCost: Double
    var cacheReadSavings: Double
    var totalCost: Double
    var model: String
    var provider: String

    enum CodingKeys: String, CodingKey {
        case inputCost = "input_cost"
        case outputCost = "output_cost"
        case cacheReadSavings = "cache_read_savings"
        case totalCost = "total_cost"
        case model, provider
    }
}

struct UsageEvent: Codable, Identifiable {
    var id: String
    var timestamp: String
    var provider: String
    var model: String
    var url: String
    var usage: TokenUsage
    var cost: CostBreakdown
}

struct ProxyConfig: Codable {
    var port: Int
    var running: Bool
}

struct DGConfig: Codable {
    var enabled: Bool
    var serverUrl: String?
    var toolsHidden: Bool
    var intelligentInterface: Bool

    enum CodingKeys: String, CodingKey {
        case enabled
        case serverUrl = "server_url"
        case toolsHidden = "tools_hidden"
        case intelligentInterface = "intelligent_interface"
    }
}

struct HealthResponse: Codable {
    var status: String
    var version: String
    var proxyRunning: Bool

    enum CodingKeys: String, CodingKey {
        case status, version
        case proxyRunning = "proxy_running"
    }
}

/// Version helpers. The app-bundle version is only available when running from
/// a real `.app` (the DMG embeds Info.plist); a dev `swift build` has no bundle
/// and returns nil. The daemon version (APIClient.coreVersion, from /health)
/// comes from Cargo and is always present once connected — so we treat *core*
/// as the authoritative "Lumen version" for display and use the bundle version
/// only to detect an app/daemon mismatch.
enum LumenVersion {
    /// nil outside a proper .app bundle (e.g. dev `swift build` / run.sh).
    static var appBundle: String? {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
    }
}

struct TrafficEntry: Codable, Identifiable {
    var id: String
    var timestamp: String
    var host: String
    var method: String
    var path: String
    var status: Int
    var requestBytes: Int
    var responseBytes: Int
    var isMonitored: Bool
    var dataCaptured: [String]
    var latencyMs: Int

    enum CodingKeys: String, CodingKey {
        case id, timestamp, host, method, path, status
        case requestBytes = "request_bytes"
        case responseBytes = "response_bytes"
        case isMonitored = "is_monitored"
        case dataCaptured = "data_captured"
        case latencyMs = "latency_ms"
    }
}

struct HostAggregate: Codable, Identifiable {
    var id: String { host }
    var host: String
    var totalRequests: Int
    var totalRequestBytes: Int
    var totalResponseBytes: Int
    var requestsMonitored: Int
    var avgLatencyMs: Double
    var lastSeen: String

    enum CodingKeys: String, CodingKey {
        case host
        case totalRequests = "total_requests"
        case totalRequestBytes = "total_request_bytes"
        case totalResponseBytes = "total_response_bytes"
        case requestsMonitored = "requests_monitored"
        case avgLatencyMs = "avg_latency_ms"
        case lastSeen = "last_seen"
    }
}

struct LapSnapshot: Codable, Identifiable {
    var id: Int { lapNumber }
    var lapNumber: Int
    var label: String
    var startedAt: String
    var endedAt: String
    var durationSecs: Double
    var inputTokens: Int
    var outputTokens: Int
    var totalTokens: Int
    var cost: Double
    var cacheSavings: Double
    var eventCount: Int
    var topModel: String?
    var tokensPerMinute: Double
    var costPerMinute: Double

    enum CodingKeys: String, CodingKey {
        case lapNumber = "lap_number"
        case label
        case startedAt = "started_at"
        case endedAt = "ended_at"
        case durationSecs = "duration_secs"
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case totalTokens = "total_tokens"
        case cost
        case cacheSavings = "cache_savings"
        case eventCount = "event_count"
        case topModel = "top_model"
        case tokensPerMinute = "tokens_per_minute"
        case costPerMinute = "cost_per_minute"
    }
}

struct RelayRoute: Codable, Identifiable {
    var id: String { prefix }
    var prefix: String
    var upstream: String
}

struct CAInfo: Codable {
    var path: String?
    var subject: String
    var issuer: String
}

struct DGStatus: Codable {
    var connected: Bool
    var subId: String?
    var serverUrl: String?
    /// Cert expiry as Unix seconds (nil if no cert / unparseable).
    var certExpiresAt: Int?
    /// "mtls" | "bearer-fallback" | "none"
    var authMode: String?
    /// True when the cert expired and we've fallen back to sync-token auth.
    /// Sync still works, but mTLS needs a reconnect to restore.
    var needsReconnect: Bool?

    enum CodingKeys: String, CodingKey {
        case connected
        case subId = "sub_id"
        case serverUrl = "server_url"
        case certExpiresAt = "cert_expires_at"
        case authMode = "auth_mode"
        case needsReconnect = "needs_reconnect"
    }

    /// True when an identity exists but its cert has lapsed (degraded mode).
    var isExpiredSession: Bool { needsReconnect == true }

    /// True only when fully healthy on mTLS.
    var isHealthy: Bool { connected && needsReconnect != true }
}

@Observable
final class APIClient {
    var stats = AggregateStats()
    var recentEvents: [UsageEvent] = []
    var proxyConfig = ProxyConfig(port: 9090, running: false)
    var dgConfig = DGConfig(enabled: false, serverUrl: nil, toolsHidden: false, intelligentInterface: false)
    var hosts: [String] = ["api.openai.com", "api.anthropic.com", "generativelanguage.googleapis.com"]
    var connected = false
    var trafficEntries: [TrafficEntry] = []
    var trafficHostAggregates: [HostAggregate] = []
    var trafficTabActive = false
    var laps: [LapSnapshot] = []
    var routes: [RelayRoute] = []
    var caInfo: CAInfo?
    var dgStatus: DGStatus?
    /// Version reported by the running lumen-core daemon (from /health). May
    /// differ from the Swift app version if an external/older daemon is attached.
    var coreVersion: String?
    /// Whether the Lumen CA is trusted in the user's login keychain. Refreshed
    /// every ~5 s on a background queue (see refreshCATrust), so the right-click
    /// menu and Settings view can read it instantly without blocking on a
    /// `security dump-trust-settings` subprocess at draw time.
    var caTrusted: Bool = false

    private let baseURL = "http://127.0.0.1:9091"
    private var pollTimer: Timer?
    private var trustCheckTimer: Timer?
    private var lastTrafficRevision: UInt64 = 0
    private let apiToken: String? = {
        let path = URL(fileURLWithPath: NSHomeDirectory())
            .appendingPathComponent(".lumen/api.token")
        return try? String(contentsOf: path, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }()

    init() {
        startPolling()
    }

    func startPolling() {
        pollTimer?.invalidate()
        trustCheckTimer?.invalidate()

        // Use .common modes so the timer keeps firing while NSPopover is open
        // (popover can hold the runloop in event-tracking mode, which would
        // otherwise pause a default-mode timer — symptom: UI stops updating
        // until the popover is closed and reopened).
        let timer = Timer(timeInterval: 1.5, repeats: true) { [weak self] _ in
            guard let self else { return }
            Task { await self.poll() }
        }
        timer.tolerance = 0.15   // lets the OS coalesce wakeups — battery friendly
        RunLoop.main.add(timer, forMode: .common)
        pollTimer = timer
        Task { await poll() }

        // CA trust check runs less often than the main poll (it spawns a
        // subprocess) and on its own dedicated timer so it's easy to tune
        // independently. First check is immediate; subsequent every 5 s.
        refreshCATrust()
        let trustTimer = Timer(timeInterval: 5.0, repeats: true) { [weak self] _ in
            self?.refreshCATrust()
        }
        trustTimer.tolerance = 1.0
        RunLoop.main.add(trustTimer, forMode: .common)
        trustCheckTimer = trustTimer
    }

    func stopPolling() {
        pollTimer?.invalidate()
        pollTimer = nil
        trustCheckTimer?.invalidate()
        trustCheckTimer = nil
    }

    /// Runs `security dump-trust-settings` on a background queue and updates
    /// `caTrusted` on the main actor. Safe to call frequently — work is
    /// effectively bounded to one subprocess spawn per call.
    func refreshCATrust() {
        DispatchQueue.global(qos: .utility).async { [weak self] in
            // Through `Shell.capture`: a trust dump is easily larger than a pipe
            // buffer, and reading it only after `waitUntilExit()` wedged the
            // subprocess permanently — every five seconds, forever, with
            // `caTrusted` frozen at whatever it last managed to read.
            let out = Shell.capture(executable: "/usr/bin/security",
                                    arguments: ["dump-trust-settings"])
            let trusted = out.localizedCaseInsensitiveContains("Lumen Local CA")
            DispatchQueue.main.async {
                self?.caTrusted = trusted
            }
        }
    }

    /// Trigger an immediate out-of-band poll, e.g. after a restart action.
    func pollNow() {
        Task { await poll() }
    }

    private func poll() async {
        await fetchStats()
        await fetchEvents()
        await fetchProxyConfig()
        await fetchHosts()
        await fetchRoutes()
        await fetchLaps()
        await fetchCAInfo()
        await fetchDGStatus()
        await fetchDGConfig()
        if coreVersion == nil {
            await fetchHealth()
        }
        if trafficTabActive {
            await fetchTrafficIfChanged()
        }
    }

    /// Fetch the daemon's reported version (and re-fetch on reconnect, since an
    /// externally-restarted daemon could be a different build).
    func fetchHealth() async {
        guard let data = await get("/health") else { return }
        if let decoded = try? JSONDecoder().decode(HealthResponse.self, from: data) {
            await MainActor.run { coreVersion = decoded.version }
        }
    }

    func fetchDGConfig() async {
        guard let data = await get("/config") else { return }
        if let decoded = try? JSONDecoder().decode(DGConfig.self, from: data) {
            await MainActor.run { dgConfig = decoded }
        }
    }

    func fetchStats() async {
        guard let data = await get("/stats") else {
            // Drop the cached daemon version on disconnect so it's re-fetched
            // on reconnect (the daemon could have restarted to a new build).
            await MainActor.run { connected = false; coreVersion = nil }
            return
        }
        if let decoded = try? JSONDecoder().decode(AggregateStats.self, from: data) {
            await MainActor.run {
                stats = decoded
                connected = true
            }
        }
    }

    func fetchEvents() async {
        guard let data = await get("/events?limit=50") else { return }
        if let decoded = try? JSONDecoder().decode([UsageEvent].self, from: data) {
            await MainActor.run { recentEvents = decoded }
        }
    }

    func fetchProxyConfig() async {
        guard let data = await get("/proxy/config") else { return }
        if let decoded = try? JSONDecoder().decode(ProxyConfig.self, from: data) {
            await MainActor.run { proxyConfig = decoded }
        }
    }

    func fetchHosts() async {
        guard let data = await get("/hosts") else { return }
        if let decoded = try? JSONDecoder().decode([String].self, from: data) {
            await MainActor.run { hosts = decoded }
        }
    }

    func clearSession() async {
        let _ = await post("/clear", body: nil)
        await fetchStats()
        await fetchEvents()
    }

    func addHost(_ host: String) async {
        let body = try? JSONSerialization.data(withJSONObject: ["host": host])
        let _ = await post("/hosts", body: body)
        await fetchHosts()
    }

    func removeHost(_ host: String) async {
        let _ = await delete("/hosts/\(host)")
        await fetchHosts()
    }

    func fetchRoutes() async {
        guard let data = await get("/routes") else { return }
        if let decoded = try? JSONDecoder().decode([RelayRoute].self, from: data) {
            await MainActor.run { routes = decoded }
        }
    }

    func addRoute(prefix: String, upstream: String) async {
        let body = try? JSONSerialization.data(withJSONObject: ["prefix": prefix, "upstream": upstream])
        let _ = await post("/routes", body: body)
        await fetchRoutes()
    }

    func removeRoute(_ prefix: String) async {
        let stripped = prefix.hasPrefix("/") ? String(prefix.dropFirst()) : prefix
        let _ = await delete("/routes/\(stripped)")
        await fetchRoutes()
    }

    func fetchCAInfo() async {
        guard let data = await get("/ca/info") else { return }
        if let decoded = try? JSONDecoder().decode(CAInfo.self, from: data) {
            await MainActor.run { caInfo = decoded }
        }
    }

    func downloadCAPEM() async -> Data? {
        return await get("/ca/pem")
    }

    private func fetchTrafficIfChanged() async {
        guard let data = await get("/traffic/revision") else { return }
        struct Rev: Codable { var revision: UInt64 }
        guard let rev = try? JSONDecoder().decode(Rev.self, from: data) else { return }
        if rev.revision == lastTrafficRevision { return }
        lastTrafficRevision = rev.revision
        await fetchTraffic()
        await fetchTrafficHosts()
    }

    func fetchTraffic(host: String? = nil, monitoredOnly: Bool = false) async {
        var path = "/traffic?limit=200"
        if let h = host { path += "&host=\(h)" }
        if monitoredOnly { path += "&monitored=true" }
        guard let data = await get(path) else { return }
        if let decoded = try? JSONDecoder().decode([TrafficEntry].self, from: data) {
            await MainActor.run { trafficEntries = decoded }
        }
    }

    func fetchTrafficHosts() async {
        guard let data = await get("/traffic/hosts") else { return }
        if let decoded = try? JSONDecoder().decode([HostAggregate].self, from: data) {
            await MainActor.run { trafficHostAggregates = decoded }
        }
    }

    func startProxy() async {
        let _ = await post("/proxy/start", body: nil)
        await fetchProxyConfig()
    }

    func createLap(label: String? = nil) async {
        var body: Data? = nil
        if let label = label {
            body = try? JSONSerialization.data(withJSONObject: ["label": label])
        }
        let _ = await post("/lap", body: body)
        await fetchLaps()
        await fetchStats()
    }

    func fetchLaps() async {
        guard let data = await get("/laps") else { return }
        if let decoded = try? JSONDecoder().decode([LapSnapshot].self, from: data) {
            await MainActor.run { laps = decoded }
        }
    }

    // MARK: - DCR OAuth Flow

    struct DCRStartResult {
        let authUrl: String
    }

    func startDCRFlow(serverUrl: String, deviceName: String) async -> DCRStartResult? {
        let body = try? JSONSerialization.data(withJSONObject: [
            "server_url": serverUrl,
            "device_name": deviceName,
        ])
        guard let data = await post("/dg/dcr", body: body) else { return nil }
        guard let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let authUrl = json["auth_url"] as? String else { return nil }
        return DCRStartResult(authUrl: authUrl)
    }

    func getDCRStatus() async -> [String: Any]? {
        guard let data = await get("/dg/dcr/status") else { return nil }
        return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    }

    func fetchDGStatus() async {
        guard let data = await get("/dg/status") else { return }
        if let decoded = try? JSONDecoder().decode(DGStatus.self, from: data) {
            await MainActor.run { dgStatus = decoded }
        }
    }

    func bootstrapDG(serverUrl: String, token: String, deviceName: String) async -> Bool {
        let body = try? JSONSerialization.data(withJSONObject: [
            "server_url": serverUrl,
            "token": token,
            "device_name": deviceName
        ])
        guard await post("/dg/bootstrap", body: body) != nil else { return false }
        await fetchDGStatus()
        return true
    }

    func disconnectDG() async -> Bool {
        guard let url = URL(string: "\(baseURL)/dg/identity") else { return false }
        var req = URLRequest(url: url)
        req.httpMethod = "DELETE"
        req.timeoutInterval = 10
        if let token = apiToken { req.setValue(token, forHTTPHeaderField: "X-Lumen-Token") }
        do {
            let (_, resp) = try await URLSession.shared.data(for: req)
            let ok = (resp as? HTTPURLResponse)?.statusCode == 200
            if ok {
                await fetchDGStatus()
                await MainActor.run {
                    var cfg = dgConfig
                    cfg.serverUrl = nil
                    dgConfig = cfg
                }
            }
            return ok
        } catch { return false }
    }

    func requestShutdown() async -> Bool {
        let result = await post("/shutdown", body: nil)
        return result != nil
    }

    func updateDGConfig(_ config: DGConfig) async {
        var changes: [String] = []
        if config.toolsHidden != dgConfig.toolsHidden {
            changes.append("tools \(config.toolsHidden ? "hidden" : "visible")")
        }
        if config.intelligentInterface != dgConfig.intelligentInterface {
            changes.append("II \(config.intelligentInterface ? "on" : "off")")
        }
        if config.enabled != dgConfig.enabled {
            changes.append("DG \(config.enabled ? "enabled" : "disabled")")
        }

        if !changes.isEmpty {
            await createLap(label: "Config: \(changes.joined(separator: ", "))")
        }

        let body = try? JSONEncoder().encode(config)
        let _ = await put("/config", body: body)
        guard let data = await get("/config") else { return }
        if let decoded = try? JSONDecoder().decode(DGConfig.self, from: data) {
            await MainActor.run { dgConfig = decoded }
        }
    }

    // MARK: - HTTP helpers

    private func get(_ path: String) async -> Data? {
        guard let url = URL(string: baseURL + path) else { return nil }
        var req = URLRequest(url: url)
        if let token = apiToken { req.setValue(token, forHTTPHeaderField: "X-Lumen-Token") }
        do {
            let (data, _) = try await URLSession.shared.data(for: req)
            return data
        } catch {
            return nil
        }
    }

    private func post(_ path: String, body: Data?) async -> Data? {
        return await request("POST", path: path, body: body)
    }

    private func put(_ path: String, body: Data?) async -> Data? {
        return await request("PUT", path: path, body: body)
    }

    private func delete(_ path: String) async -> Data? {
        return await request("DELETE", path: path, body: nil)
    }

    private func request(_ method: String, path: String, body: Data?) async -> Data? {
        guard let url = URL(string: baseURL + path) else { return nil }
        var req = URLRequest(url: url)
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let token = apiToken { req.setValue(token, forHTTPHeaderField: "X-Lumen-Token") }
        req.httpBody = body
        do {
            let (data, _) = try await URLSession.shared.data(for: req)
            return data
        } catch {
            return nil
        }
    }
}
