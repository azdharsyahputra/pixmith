import AppKit
import Foundation
@testable import PixmithKit
import PixmithCore

// pixmith-qa: headless end-to-end scenarios against the real app code (SpriteModel, the canvas view's mouse handlers,
// the file actions, ...).
//
//   pixmith-qa                   run every scenario, each in its own process, and summarize
//   pixmith-qa --list            print scenario names
//   pixmith-qa --run NAME        run one scenario in this process (exit status 1 on failure)
//   pixmith-qa --all-in-process  run everything in this process (faster, but one crash ends the run)
//   pixmith-qa PATTERN           run only the scenarios whose name contains PATTERN
//
// Environment: PIXMITH_QA_OUT (snapshot directory, default ./qa-out), PIXMITH_QA_TIMEOUT (seconds per scenario).

// The core and the app write recovery copies and preferences below the home directory; keep QA runs out of the real one.
let qaHome = FileManager.default.temporaryDirectory.appendingPathComponent("pixmith-qa-home")
try? FileManager.default.createDirectory(at: qaHome, withIntermediateDirectories: true)
setenv("HOME", qaHome.path, 1)
setenv("PIXMITH_SUPPORT_DIR", qaHome.appendingPathComponent("Library/Application Support").path, 1)

registerScenarios()

let args = Array(CommandLine.arguments.dropFirst())

func runInProcess(_ list: [Scenario]) -> Int32 {
    let app = NSApplication.shared
    app.setActivationPolicy(.prohibited)
    var failed: Int32 = 0
    for s in list {
        QA.scenario = s.name
        let before = QA.failures.count
        s.body()
        if QA.failures.count != before { failed = 1 }
        print("\(QA.failures.count == before ? "ok  " : "FAIL") \(s.name)")
        fflush(stdout)
    }
    print("checks: \(QA.checks), failures: \(QA.failures.count)")
    return failed
}

if args.first == "--list" {
    scenarios.forEach { print($0.name) }
    exit(0)
}

if args.first == "--run", args.count > 1 {
    guard let s = scenarios.first(where: { $0.name == args[1] }) else {
        print("unknown scenario \(args[1])")
        exit(2)
    }
    let status = runInProcess([s])
    try? FileManager.default.removeItem(at: tmpDir)
    exit(status)
}

if args.first == "--all-in-process" {
    exit(runInProcess(scenarios))
}

// Parent: one child process per scenario, so a crash (the core aborts on panic) only fails that scenario.
let filter = args.first
let selected = scenarios.filter { filter == nil || $0.name.contains(filter!) }
let timeout = Double(ProcessInfo.processInfo.environment["PIXMITH_QA_TIMEOUT"] ?? "") ?? 180
var failedNames: [String] = []
var totalChecks = 0
for s in selected {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
    p.arguments = ["--run", s.name]
    let pipe = Pipe()
    p.standardOutput = pipe
    p.standardError = pipe
    do { try p.run() } catch {
        print("FAIL \(s.name): cannot start child: \(error)")
        failedNames.append(s.name)
        continue
    }
    let deadline = Date(timeIntervalSinceNow: timeout)
    var timedOut = false
    var data = Data()
    let reader = DispatchQueue(label: "read")
    let group = DispatchGroup()
    group.enter()
    reader.async {
        data = pipe.fileHandleForReading.readDataToEndOfFile()
        group.leave()
    }
    while p.isRunning {
        if Date() > deadline {
            timedOut = true
            p.terminate()
            break
        }
        Thread.sleep(forTimeInterval: 0.05)
    }
    p.waitUntilExit()
    group.wait()
    let out = String(decoding: data, as: UTF8.self)
    let lines = out.split(separator: "\n").map(String.init)
    let verbose = ProcessInfo.processInfo.environment["PIXMITH_QA_VERBOSE"] != nil
    for l in lines where (l.hasPrefix("FAIL ") && l.contains(": ")) || (verbose && l.hasPrefix("note ")) { print(l) }
    if let c = lines.last(where: { $0.hasPrefix("checks: ") }),
       let n = Int(c.dropFirst(8).split(separator: ",").first ?? "") { totalChecks += n }
    let crashed = p.terminationReason == .uncaughtSignal
    if timedOut || crashed || p.terminationStatus != 0 {
        failedNames.append(s.name)
        if timedOut { print("FAIL \(s.name): timed out after \(Int(timeout))s") }
        else if crashed { print("FAIL \(s.name): crashed (signal \(p.terminationStatus))\n" + lines.suffix(5).joined(separator: "\n")) }
        else if !lines.contains(where: { $0.hasPrefix("FAIL ") && $0.contains(": ") }) { print("FAIL \(s.name): exit status \(p.terminationStatus)\n" + lines.suffix(8).joined(separator: "\n")) }
    }
    print("\(failedNames.last == s.name ? "FAIL" : "ok  ") \(s.name)")
    fflush(stdout)
}
print("\nscenarios: \(selected.count), failed: \(failedNames.count), checks: \(totalChecks)")
if !failedNames.isEmpty { print("failed: " + failedNames.joined(separator: ", ")) }
exit(failedNames.isEmpty ? 0 : 1)
