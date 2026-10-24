import AppKit

/// The `pixmith` command: the bundled `pixmith-cli` (Contents/MacOS/pixmith-cli) linked onto the PATH, like
/// VS Code's "Install 'code' command in PATH". Help > Install Command Line Tool… / Uninstall….
enum CommandLineTool {
    static let command = "pixmith"
    /// System-wide location; needs an administrator password once.
    static let systemDir = URL(fileURLWithPath: "/usr/local/bin")
    /// Per-user location that needs no password; it must be on the user's PATH.
    static var userDir: URL { FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin") }

    /// The CLI inside this app bundle, if the build includes it.
    static var bundled: URL? {
        let url = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/pixmith-cli")
        return FileManager.default.isExecutableFile(atPath: url.path) ? url : nil
    }

    /// Where the command is installed now (a symlink to this app's CLI), if anywhere.
    static var installedLink: URL? {
        [systemDir, userDir].map { $0.appendingPathComponent(command) }.first { link in
            (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path))?.hasSuffix("/pixmith-cli") == true
        }
    }

    static func install() {
        guard let cli = bundled else {
            alert("The command line tool is not part of this build.",
                  "Build the app with ./build.sh or scripts/package.sh, which put pixmith-cli inside Pixmith.app.")
            return
        }
        let ask = NSAlert()
        ask.messageText = "Install the “\(command)” command?"
        ask.informativeText = """
            It runs Pixmith's command line tool from Terminal, for example:
            \(command) info hero.pxmt
            \(command) export hero.pxmt --gif hero.gif --scale 4
            \(command) convert hero.aseprite hero.pxmt

            For all users it goes in /usr/local/bin (asks for your password once). For you only it goes in \
            ~/.local/bin, which must be on your PATH.
            """
        ask.addButton(withTitle: "Install for All Users")
        ask.addButton(withTitle: "Install for Me Only")
        ask.addButton(withTitle: "Cancel")
        switch ask.runModal() {
        case .alertFirstButtonReturn: installSystemWide(cli)
        case .alertSecondButtonReturn: installForUser(cli)
        default: break
        }
    }

    static func uninstall() {
        guard let link = installedLink else {
            alert("The “\(command)” command is not installed.", "")
            return
        }
        if link.deletingLastPathComponent() == systemDir {
            runAsAdmin("rm -f \(shellQuote(link.path))", done: "The “\(command)” command was removed.")
        } else {
            do {
                try FileManager.default.removeItem(at: link)
                alert("The “\(command)” command was removed.", "")
            } catch {
                alert("Could not remove \(link.path).", error.localizedDescription)
            }
        }
    }

    private static func installSystemWide(_ cli: URL) {
        let link = systemDir.appendingPathComponent(command)
        let script = "mkdir -p \(shellQuote(systemDir.path)) && ln -sf \(shellQuote(cli.path)) \(shellQuote(link.path))"
        runAsAdmin(script, done: "Installed. Open a new Terminal window and run “\(command) --help”.")
    }

    private static func installForUser(_ cli: URL) {
        let link = userDir.appendingPathComponent(command)
        do {
            try FileManager.default.createDirectory(at: userDir, withIntermediateDirectories: true)
            if (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)) != nil
                || FileManager.default.fileExists(atPath: link.path) {
                try FileManager.default.removeItem(at: link)
            }
            try FileManager.default.createSymbolicLink(at: link, withDestinationURL: cli)
        } catch {
            alert("Could not install the command.", error.localizedDescription)
            return
        }
        let path = ProcessInfo.processInfo.environment["PATH"] ?? ""
        let onPath = path.split(separator: ":").contains { $0 == userDir.path }
        alert("Installed in ~/.local/bin.", onPath
            ? "Open a new Terminal window and run “\(command) --help”."
            : "Add it to your PATH, for example by adding this line to ~/.zshrc:\nexport PATH=\"$HOME/.local/bin:$PATH\"")
    }

    /// Runs `script` with administrator rights through the standard macOS password prompt.
    private static func runAsAdmin(_ script: String, done: String) {
        let escaped = script.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        var error: NSDictionary?
        NSAppleScript(source: "do shell script \"\(escaped)\" with administrator privileges")?.executeAndReturnError(&error)
        if let error {
            // -128: the user cancelled the password prompt.
            if (error[NSAppleScript.errorNumber] as? Int) != -128 {
                alert("Could not change \(systemDir.path).", (error[NSAppleScript.errorMessage] as? String) ?? "")
            }
        } else {
            alert(done, "")
        }
    }

    private static func shellQuote(_ s: String) -> String { "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'" }

    private static func alert(_ title: String, _ text: String) {
        let a = NSAlert()
        a.messageText = title
        a.informativeText = text
        a.runModal()
    }
}
