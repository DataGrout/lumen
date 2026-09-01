import Foundation

/// Running a command-line tool and capturing what it said, without deadlocking.
///
/// The obvious spelling of this is wrong, and was wrong in four places here:
///
///     try proc.run()
///     proc.waitUntilExit()                                  // ← waits for exit
///     let out = pipe.fileHandleForReading.readDataToEndOfFile()   // ← drains after
///
/// A pipe holds ~64KB. Once a child fills it, `write(2)` blocks until somebody
/// reads — and nobody will, because the parent is in `waitUntilExit()` waiting
/// for the child to exit. Neither side can move again, ever.
///
/// Observed 2026-08-21: `networksetup -listnetworkserviceorder` (long on a
/// machine with VPN adapters) wedged for **four hours**, blocked in
/// `__write_nocancel`, while the app's MAIN THREAD sat in `waitUntilExit` — a
/// permanent beach ball on a Lumen that was otherwise still capturing traffic,
/// because the daemon is a separate process and only the UI was dead.
///
/// The fix is to drain the pipe *while* the child runs, so the child can never
/// block on a full buffer. A timeout is here too: a tool can hang for reasons
/// that have nothing to do with pipes (`networksetup` talks to configd, which a
/// VPN in a bad state can stall), and no UI-adjacent call should be able to wait
/// on that forever.
enum Shell {
    /// Combined stdout+stderr of `/bin/sh -c command`, or `""` on failure.
    ///
    /// Blocks the calling thread for up to `timeout`, so **do not call this on the
    /// main thread** — bounded is not the same as fast. `SystemProxy`'s callers
    /// hop to a background queue first.
    static func capture(_ command: String, timeout: TimeInterval = 5.0) -> String {
        let proc = Process()
        let pipe = Pipe()
        proc.executableURL = URL(fileURLWithPath: "/bin/sh")
        proc.arguments = ["-c", command]
        proc.standardOutput = pipe
        proc.standardError = pipe

        do {
            try proc.run()
        } catch {
            return ""
        }

        // Drain concurrently. `readDataToEndOfFile` returns when the write end
        // closes, which happens when the child exits — so this reader is what
        // lets the child finish at all when its output is large.
        var captured = Data()
        let finished = DispatchSemaphore(value: 0)

        DispatchQueue.global(qos: .userInitiated).async {
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            captured = data
            finished.signal()
        }

        if finished.wait(timeout: .now() + timeout) == .timedOut {
            // Still writing (or wedged elsewhere) past our patience. Killing the
            // child closes the pipe, which unblocks the reader we just abandoned.
            NSLog("[Lumen] shell timeout after %.1fs: %@", timeout, command)
            proc.terminate()

            // A moment for the reader to observe EOF, then give up on the output
            // rather than the process: a partial answer read after termination is
            // not one to trust.
            _ = finished.wait(timeout: .now() + 1.0)
            return ""
        }

        // The child has closed its output, so this returns immediately — the
        // ordering that makes the whole thing safe.
        proc.waitUntilExit()

        return String(data: captured, encoding: .utf8) ?? ""
    }

    /// As `capture`, for a tool invoked directly rather than through a shell.
    /// Avoids quoting entirely, which is why the trust dump uses it.
    static func capture(
        executable: String,
        arguments: [String],
        timeout: TimeInterval = 5.0
    ) -> String {
        let quoted = ([executable] + arguments)
            .map { "'" + $0.replacingOccurrences(of: "'", with: "'\\''") + "'" }
            .joined(separator: " ")

        return capture(quoted, timeout: timeout)
    }
}
