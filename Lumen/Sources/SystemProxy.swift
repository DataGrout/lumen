import Foundation
import ServiceManagement

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

    /// Keep Lumen a login item for as long as it manages the system proxy.
    ///
    /// This closes the worst failure this app can cause. The proxy setting lives in
    /// /Library/Preferences/SystemConfiguration/preferences.plist and survives a
    /// reboot; Lumen did not start itself, so nothing answered on the port. Every
    /// app honouring the system proxy — Chrome included — then had no internet at
    /// all, on a machine whose link, DHCP and DNS were perfectly healthy. Reported
    /// 2026-08-31, recurring on every boot until the user found the setting by hand.
    ///
    /// A terminate handler cannot close this, because it does not run on a crash, a
    /// force-quit, or a power cut. The durable guarantee is the other direction:
    /// whatever serves the port comes back whenever the setting does.
    static func syncLoginItem(managesProxy: Bool) {
        let service = SMAppService.mainApp

        do {
            switch (managesProxy, service.status) {
            case (true, .enabled), (false, .notRegistered), (false, .notFound):
                break
            case (true, _):
                try service.register()
                NSLog("[Lumen] Registered as a login item — the system proxy needs us running")
            case (false, _):
                // No longer touching the system proxy, so no longer a reason to
                // launch ourselves at login.
                try service.unregister()
                NSLog("[Lumen] Unregistered as a login item — no proxy left to serve")
            }
        } catch {
            // Not fatal for this session, but loud: it means the reboot failure
            // above is still reachable.
            NSLog("[Lumen] Could not update login-item registration: %@ — a reboot may leave the system proxy aimed at a dead port",
                  error.localizedDescription)
        }
    }

    /// Our proxy on exactly one service: the one macOS is actually routing through.
    ///
    /// Applying to the new primary without clearing the old one leaves the machine
    /// carrying a proxy on a service nothing serves the moment Lumen is not
    /// running — and users switch constantly (dock, hotspot, VPN). Applied before
    /// the others are cleared, so there is no window with no proxy anywhere.
    @discardableResult
    static func reconcile(port: Int) async -> String {
        let primary = activeInterface()

        if !isOurProxy(on: primary, port: port) {
            _ = await enable(port: port, interface: primary)
        }

        for service in allServices()
        where service != primary && isOurProxy(on: service, port: port) {
            NSLog("[Lumen] Clearing stale system proxy from %@ (primary is %@)", service, primary)
            _ = await disable(interface: service)
        }

        return primary
    }

    /// Every network service macOS knows about, in listing order.
    static func allServices() -> [String] {
        runSync("networksetup -listallnetworkservices")
            .components(separatedBy: "\n")
            .dropFirst()                       // header line
            .map { $0.trimmingCharacters(in: .whitespaces) }
            // A leading '*' marks a disabled service; the name excludes it, and
            // networksetup still accepts the name, so strip rather than skip.
            .map { $0.hasPrefix("*") ? String($0.dropFirst()) : $0 }
            .filter { !$0.isEmpty }
    }

    /// Turn OFF our proxy on every service that currently points at it.
    ///
    /// The teardown used to clear only the service that happened to be primary at
    /// the time, which leaks: set the proxy on Wi-Fi, plug in ethernet, quit, and
    /// Wi-Fi is left aimed at a port nothing is listening on. The user then has no
    /// internet in every app that honours the system proxy — Chrome included —
    /// with a healthy link, DHCP and DNS, so nothing about the failure points at
    /// us. Reported 2026-08-31 exactly that way.
    ///
    /// Matched on 127.0.0.1 AND our port, never on "enabled": a corporate or
    /// debugging proxy the user configured themselves must survive us quitting.
    @discardableResult
    static func disableOurProxyEverywhere(port: Int) async -> [String] {
        var cleared: [String] = []

        for service in allServices() where isOurProxy(on: service, port: port) {
            if await disable(interface: service) {
                cleared.append(service)
            }
        }

        return cleared
    }

    /// Is the proxy on this service ours — 127.0.0.1 on the port we serve?
    static func isOurProxy(on service: String, port: Int) -> Bool {
        let output = runSync("networksetup -getwebproxy \"\(service)\"")
        return output.contains("Enabled: Yes")
            && output.contains("127.0.0.1")
            && output.contains("Port: \(port)")
    }

    static func isEnabled(interface: String = "Wi-Fi") -> Bool {
        let output = runSync("networksetup -getwebproxy \"\(interface)\"")
        return output.contains("Enabled: Yes") && output.contains("127.0.0.1")
    }

    /// The service macOS actually routes through — asked of the OS, not guessed.
    ///
    /// This scanned services in listing order and returned the first one holding
    /// ANY ip, which is a different question: a laptop with Wi-Fi associated and
    /// a USB ethernet dongle plugged in has two, and the proxy landed on
    /// whichever sorted first. Observed exactly that — `networksetup` reported
    /// the proxy enabled on Wi-Fi while `scutil` (what apps consult) reported
    /// none, because the primary interface was the dongle. Every request left
    /// unproxied and capture silently stopped mid-session.
    static func activeInterface() -> String {
        if let service = primaryService() {
            return service
        }

        // Fallback: the old first-with-an-ip scan, for the case where scutil
        // gives nothing useful (no route at all).
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

    /// `scutil`'s primary interface (a BSD device like `en6`), mapped back to the
    /// service name `networksetup` speaks (`USB 10/100/1000 LAN`).
    static func primaryService() -> String? {
        let state = runSync("echo 'show State:/Network/Global/IPv4' | scutil")

        guard let device = state
            .components(separatedBy: "\n")
            .first(where: { $0.contains("PrimaryInterface") })?
            .components(separatedBy: ":").last?
            .trimmingCharacters(in: .whitespaces),
            !device.isEmpty
        else {
            return nil
        }

        // `-listnetworkserviceorder` pairs each service name with its device:
        //   (6) USB 10/100/1000 LAN
        //   (Hardware Port: USB 10/100/1000 LAN, Device: en6)
        let order = runSync("networksetup -listnetworkserviceorder")
        let lines = order.components(separatedBy: "\n")

        for (index, line) in lines.enumerated() where line.contains("Device: \(device))") {
            guard index > 0 else { continue }

            let nameLine = lines[index - 1].trimmingCharacters(in: .whitespaces)

            // Strip the "(6) " ordinal prefix.
            if let range = nameLine.range(of: ") ") {
                return String(nameLine[range.upperBound...])
            }
        }

        return nil
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

    /// Every one of these tools is read through `Shell.capture`, which drains the
    /// pipe while the child runs.
    ///
    /// This used to wait for the child and read afterwards, which deadlocks the
    /// moment a tool outproduces the pipe buffer:
    /// `networksetup -listnetworkserviceorder` on a machine with VPN adapters
    /// wedged for four hours with the app's main thread inside `waitUntilExit`.
    /// See `Shell` for the mechanism.
    private static func runSync(_ command: String) -> String {
        Shell.capture(command)
    }
}
