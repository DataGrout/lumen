import Foundation

enum SystemProxy {
    static func enable(port: Int, interface: String = "Wi-Fi") async -> Bool {
        // Try without admin prompt first — works on most admin accounts
        let directOk = await runDirect([
            ["/usr/sbin/networksetup", "-setwebproxy",            interface, "127.0.0.1", "\(port)"],
            ["/usr/sbin/networksetup", "-setsecurewebproxy",      interface, "127.0.0.1", "\(port)"],
            ["/usr/sbin/networksetup", "-setwebproxystate",       interface, "on"],
            ["/usr/sbin/networksetup", "-setsecurewebproxystate", interface, "on"],
        ])
        if directOk { return true }
        // Fall back to admin prompt only when direct access is denied
        let iface = shellEscape(interface)
        return await runWithAdmin([
            "networksetup -setwebproxy \(iface) 127.0.0.1 \(port)",
            "networksetup -setsecurewebproxy \(iface) 127.0.0.1 \(port)",
            "networksetup -setwebproxystate \(iface) on",
            "networksetup -setsecurewebproxystate \(iface) on",
        ].joined(separator: " && "))
    }

    static func disable(interface: String = "Wi-Fi") async -> Bool {
        let directOk = await runDirect([
            ["/usr/sbin/networksetup", "-setwebproxystate",       interface, "off"],
            ["/usr/sbin/networksetup", "-setsecurewebproxystate", interface, "off"],
        ])
        if directOk { return true }
        let iface = shellEscape(interface)
        return await runWithAdmin([
            "networksetup -setwebproxystate \(iface) off",
            "networksetup -setsecurewebproxystate \(iface) off",
        ].joined(separator: " && "))
    }

    static func isEnabled(interface: String = "Wi-Fi") -> Bool {
        let output = runSync("networksetup -getwebproxy \"\(interface)\"")
        return output.contains("Enabled: Yes") && output.contains("127.0.0.1")
    }

    static func activeInterface() -> String {
        let output = runSync("networksetup -listallnetworkservices")
        let services = output.components(separatedBy: "\n")
            .filter { !$0.contains("*") && !$0.isEmpty }
            .dropFirst()

        for service in services {
            let trimmed = service.trimmingCharacters(in: .whitespaces)
            let status = runSync("networksetup -getinfo \"\(trimmed)\"")
            if status.contains("IP address:") && !status.contains("IP address: none") {
                return trimmed
            }
        }

        return "Wi-Fi"
    }

    private static func shellEscape(_ s: String) -> String {
        let escaped = s.replacingOccurrences(of: "'", with: "'\\''")
        return "'\(escaped)'"
    }

    private static func runDirect(_ commands: [[String]]) async -> Bool {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                for args in commands {
                    let proc = Process()
                    proc.executableURL = URL(fileURLWithPath: args[0])
                    proc.arguments = Array(args.dropFirst())
                    proc.standardOutput = FileHandle.nullDevice
                    proc.standardError = FileHandle.nullDevice
                    do {
                        try proc.run()
                        proc.waitUntilExit()
                        if proc.terminationStatus != 0 {
                            continuation.resume(returning: false)
                            return
                        }
                    } catch {
                        continuation.resume(returning: false)
                        return
                    }
                }
                continuation.resume(returning: true)
            }
        }
    }

    private static func runWithAdmin(_ command: String) async -> Bool {
        let escaped = command
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        let script = "do shell script \"\(escaped)\" with administrator privileges"

        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let proc = Process()
                proc.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
                proc.arguments = ["-e", script]
                do {
                    try proc.run()
                    proc.waitUntilExit()
                    continuation.resume(returning: proc.terminationStatus == 0)
                } catch {
                    continuation.resume(returning: false)
                }
            }
        }
    }

    private static func runSync(_ command: String) -> String {
        let proc = Process()
        let pipe = Pipe()
        proc.executableURL = URL(fileURLWithPath: "/bin/sh")
        proc.arguments = ["-c", command]
        proc.standardOutput = pipe
        proc.standardError = pipe
        do {
            try proc.run()
            proc.waitUntilExit()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            return String(data: data, encoding: .utf8) ?? ""
        } catch {
            return ""
        }
    }
}
