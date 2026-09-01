import Foundation

final class DaemonManager {
    private var process: Process?
    private var intentionalStop = false
    private let apiPort: Int

    /// Consecutive non-zero exits. A daemon that cannot start at all used to
    /// restart every two seconds forever with its output discarded, so the app
    /// showed "not running" and the Restart button appeared to do nothing —
    /// it was working, and failing again, invisibly.
    private var consecutiveFailures = 0

    /// Why the daemon last exited, if it exited badly. Surfaced in the UI so a
    /// user can say what went wrong instead of only that nothing happened.
    private(set) var lastFailureReason: String?

    /// Where the daemon's output goes. Previously /dev/null, which meant a
    /// start-up failure left no record anywhere on the machine.
    static var logURL: URL {
        let dir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".lumen")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("daemon.log")
    }

    var isRunning: Bool {
        if process?.isRunning == true {
            return true
        }
        // Check if an external instance is running (e.g. started by run.sh)
        return isExternallyRunning
    }

    private var isExternallyRunning: Bool {
        guard let url = URL(string: "http://127.0.0.1:\(apiPort)/health") else { return false }
        var request = URLRequest(url: url, timeoutInterval: 0.5)
        request.httpMethod = "GET"
        let semaphore = DispatchSemaphore(value: 0)
        var ok = false
        let task = URLSession.shared.dataTask(with: request) { _, response, _ in
            if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                ok = true
            }
            semaphore.signal()
        }
        task.resume()
        semaphore.wait()
        return ok
    }

    init(apiPort: Int = 9091) {
        self.apiPort = apiPort
    }

    func start() {
        if process?.isRunning == true { return }

        if isExternallyRunning {
            NSLog("[Lumen] External lumen-core already running, attaching")
            return
        }

        intentionalStop = false

        let binaryPath = findBinary()
        guard FileManager.default.fileExists(atPath: binaryPath) else {
            NSLog("[Lumen] lumen-core binary not found at %@", binaryPath)
            return
        }

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: binaryPath)
        proc.environment = ProcessInfo.processInfo.environment

        // Keep the daemon's output. It is the only account of why a start failed,
        // and discarding it is why an Intel tester could report nothing beyond
        // "it never starts". Truncated each launch so it stays a diagnosis of the
        // current attempt rather than an unbounded file.
        let logURL = Self.logURL
        FileManager.default.createFile(atPath: logURL.path, contents: nil)

        if let handle = try? FileHandle(forWritingTo: logURL) {
            proc.standardOutput = handle
            proc.standardError = handle
        } else {
            proc.standardOutput = FileHandle.nullDevice
            proc.standardError = FileHandle.nullDevice
        }

        proc.terminationHandler = { [weak self] p in
            guard let self = self else { return }

            let tail = Self.recentLog()
            NSLog("[Lumen] lumen-core exited with code %d. Output: %@", p.terminationStatus, tail)

            // Safety: disable system proxy if daemon crashes unexpectedly
            if !self.intentionalStop && p.terminationStatus != 0 {
                self.consecutiveFailures += 1
                self.lastFailureReason = tail.isEmpty
                    ? "exited with code \(p.terminationStatus) and produced no output"
                    : tail

                let iface = SystemProxy.activeInterface()
                if SystemProxy.isEnabled(interface: iface) {
                    NSLog("[Lumen] Daemon crashed — disabling system proxy for safety")
                    Self.disableProxySync(interface: iface)
                }

                // Back off, and stop after a few tries. A daemon that cannot start
                // will not start on the fourth attempt either, and retrying every
                // two seconds forever hid the failure rather than recovering from
                // it — the log below is the point of stopping.
                if self.consecutiveFailures >= 4 {
                    NSLog(
                        "[Lumen] lumen-core failed %d times, giving up. See %@",
                        self.consecutiveFailures, Self.logURL.path
                    )
                    return
                }

                let delay = Double(self.consecutiveFailures) * 2.0
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                    self.start()
                }
            } else {
                self.consecutiveFailures = 0
            }
        }

        do {
            try proc.run()
            process = proc
            lastFailureReason = nil
            NSLog("[Lumen] lumen-core started (pid %d)", proc.processIdentifier)
        } catch {
            // `proc.run()` throwing is a different failure from the daemon exiting:
            // the binary could not be executed at all (permissions, quarantine, a
            // missing slice). Recording it distinctly is what tells those apart.
            lastFailureReason = "could not execute \(binaryPath): \(error.localizedDescription)"
            consecutiveFailures += 1
            NSLog("[Lumen] Failed to start lumen-core: %@", error.localizedDescription)
        }
    }

    /// The tail of the daemon's output, for reporting a failed start.
    static func recentLog(maxLines: Int = 12) -> String {
        guard let data = try? Data(contentsOf: logURL),
              let text = String(data: data, encoding: .utf8) else { return "" }

        return text
            .split(separator: "\n", omittingEmptySubsequences: true)
            .suffix(maxLines)
            .joined(separator: "\n")
    }

    /// Clear the failure state so the user's explicit Restart is a real retry
    /// rather than one more attempt against an exhausted budget.
    func resetFailures() {
        consecutiveFailures = 0
        lastFailureReason = nil
    }

    func stop() {
        intentionalStop = true
        guard let proc = process, proc.isRunning else {
            process = nil
            return
        }

        if requestShutdownSync() {
            let deadline = Date().addingTimeInterval(3)
            while proc.isRunning && Date() < deadline {
                Thread.sleep(forTimeInterval: 0.1)
            }
        }

        if proc.isRunning {
            NSLog("[Lumen] Graceful shutdown timed out, sending SIGTERM")
            proc.terminate()
            let deadline = Date().addingTimeInterval(2)
            while proc.isRunning && Date() < deadline {
                Thread.sleep(forTimeInterval: 0.1)
            }
        }

        if proc.isRunning {
            NSLog("[Lumen] SIGTERM timed out, sending SIGKILL")
            kill(proc.processIdentifier, SIGKILL)
            proc.waitUntilExit()
        }

        process = nil
        NSLog("[Lumen] lumen-core stopped")
    }

    func waitForHealthy(timeout: TimeInterval = 5.0) -> Bool {
        guard let url = URL(string: "http://127.0.0.1:\(apiPort)/health") else { return false }
        let deadline = Date().addingTimeInterval(timeout)
        var backoff: TimeInterval = 0.1

        while Date() < deadline {
            var request = URLRequest(url: url, timeoutInterval: 1)
            request.httpMethod = "GET"

            let semaphore = DispatchSemaphore(value: 0)
            var success = false

            let task = URLSession.shared.dataTask(with: request) { _, response, _ in
                if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                    success = true
                }
                semaphore.signal()
            }
            task.resume()
            semaphore.wait()

            if success { return true }
            Thread.sleep(forTimeInterval: backoff)
            backoff = min(backoff * 1.5, 1.0)
        }

        return false
    }

    private func requestShutdownSync() -> Bool {
        guard let url = URL(string: "http://127.0.0.1:\(apiPort)/shutdown") else { return false }
        var request = URLRequest(url: url, timeoutInterval: 2)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        let semaphore = DispatchSemaphore(value: 0)
        var success = false

        let task = URLSession.shared.dataTask(with: request) { _, response, _ in
            if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                success = true
            }
            semaphore.signal()
        }
        task.resume()
        semaphore.wait()

        if success {
            NSLog("[Lumen] Graceful shutdown requested via API")
        }
        return success
    }

    private static func disableProxySync(interface: String) {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
        proc.arguments = ["-setwebproxystate", interface, "off"]
        try? proc.run()
        proc.waitUntilExit()

        let proc2 = Process()
        proc2.executableURL = URL(fileURLWithPath: "/usr/sbin/networksetup")
        proc2.arguments = ["-setsecurewebproxystate", interface, "off"]
        try? proc2.run()
        proc2.waitUntilExit()
    }

    private func findBinary() -> String {
        if let bundled = Bundle.main.path(forAuxiliaryExecutable: "lumen-core") {
            return bundled
        }

        let execDir = Bundle.main.executableURL?.deletingLastPathComponent().path ?? ""
        let sibling = "\(execDir)/lumen-core"
        if FileManager.default.fileExists(atPath: sibling) {
            return sibling
        }

        let devPath = URL(fileURLWithPath: #file)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("lumen-core/target/release/lumen-core")
            .path
        if FileManager.default.fileExists(atPath: devPath) {
            return devPath
        }

        let debugPath = URL(fileURLWithPath: #file)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("lumen-core/target/debug/lumen-core")
            .path

        return debugPath
    }
}
