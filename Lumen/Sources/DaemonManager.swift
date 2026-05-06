import Foundation

final class DaemonManager {
    private var process: Process?
    private var intentionalStop = false
    private let apiPort: Int

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
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice

        proc.terminationHandler = { [weak self] p in
            guard let self = self else { return }
            NSLog("[Lumen] lumen-core exited with code %d", p.terminationStatus)

            // Safety: disable system proxy if daemon crashes unexpectedly
            if !self.intentionalStop && p.terminationStatus != 0 {
                let iface = SystemProxy.activeInterface()
                if SystemProxy.isEnabled(interface: iface) {
                    NSLog("[Lumen] Daemon crashed — disabling system proxy for safety")
                    Self.disableProxySync(interface: iface)
                }

                DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                    self.start()
                }
            }
        }

        do {
            try proc.run()
            process = proc
            NSLog("[Lumen] lumen-core started (pid %d)", proc.processIdentifier)
        } catch {
            NSLog("[Lumen] Failed to start lumen-core: %@", error.localizedDescription)
        }
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
