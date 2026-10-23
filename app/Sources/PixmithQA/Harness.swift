import AppKit
import Foundation
@testable import PixmithKit
import PixmithCore

// MARK: - Assertions

enum QA {
    static var scenario = ""
    static var checks = 0
    static var failures: [String] = []
    static var outDir = ProcessInfo.processInfo.environment["PIXMITH_QA_OUT"] ?? "qa-out"

    static func fail(_ message: String, _ file: String, _ line: UInt) {
        let text = "FAIL \(scenario): \(message) (\(file.split(separator: "/").last ?? ""):\(line))"
        failures.append(text)
        print(text)
        fflush(stdout)
    }
}

func expect(_ cond: @autoclosure () -> Bool, _ message: @autoclosure () -> String = "expectation failed",
            file: String = #fileID, line: UInt = #line) {
    QA.checks += 1
    if !cond() { QA.fail(message(), file, line) }
}

func expectEq<T: Equatable>(_ a: @autoclosure () -> T, _ b: @autoclosure () -> T, _ message: String = "",
                            file: String = #fileID, line: UInt = #line) {
    QA.checks += 1
    let (x, y) = (a(), b())
    if x != y { QA.fail("\(message.isEmpty ? "" : message + ": ")got \(x), expected \(y)", file, line) }
}

/// Pixel colors in failure messages.
func hex(_ c: UInt32) -> String { String(format: "#%08X", c) }

func expectPixel(_ x: Int, _ y: Int, _ color: UInt32, _ message: String = "", frame: Int? = nil,
                 file: String = #fileID, line: UInt = #line) {
    QA.checks += 1
    let got = SpriteModel.shared.pixel(x, y, frame: frame)
    if got != color { QA.fail("\(message.isEmpty ? "" : message + ": ")pixel (\(x),\(y)) is \(hex(got)), expected \(hex(color))", file, line) }
}

// MARK: - Scenario registry

struct Scenario {
    let name: String
    let body: () -> Void
}

var scenarios: [Scenario] = []

func scenario(_ name: String, _ body: @escaping () -> Void) {
    scenarios.append(Scenario(name: name, body: body))
}

// MARK: - Model helpers

let RED: UInt32 = 0xFF0000FF, GREEN: UInt32 = 0x00FF00FF, BLUE: UInt32 = 0x0000FFFF
let BLACK: UInt32 = 0x000000FF, WHITE: UInt32 = 0xFFFFFFFF, CLEAR: UInt32 = 0

extension SpriteModel {
    /// Composite of one pixel, 0xRRGGBBAA.
    func pixel(_ x: Int, _ y: Int, frame f: Int? = nil) -> UInt32 {
        let buf = render(frame: f ?? frame)
        guard x >= 0, y >= 0, x < width, y < height else { return 0xDEADBEEF }
        let i = (y * width + x) * 4
        return UInt32(buf[i]) << 24 | UInt32(buf[i + 1]) << 16 | UInt32(buf[i + 2]) << 8 | UInt32(buf[i + 3])
    }

    func render(frame f: Int? = nil) -> [UInt8] {
        var buf = [UInt8](repeating: 0, count: width * height * 4)
        let ok = pixmith_render(doc, UInt32(f ?? frame), &buf, buf.count)
        expect(ok, "pixmith_render failed")
        return buf
    }

    /// Every pixel of the composite as 0xRRGGBBAA.
    func pixels(frame f: Int? = nil) -> [UInt32] {
        let b = render(frame: f)
        return stride(from: 0, to: b.count, by: 4).map { UInt32(b[$0]) << 24 | UInt32(b[$0 + 1]) << 16 | UInt32(b[$0 + 2]) << 8 | UInt32(b[$0 + 3]) }
    }

    /// Coordinates of all pixels with alpha, in reading order.
    func opaque(frame f: Int? = nil) -> [(Int, Int)] {
        let p = pixels(frame: f)
        return p.enumerated().filter { $0.element & 0xFF != 0 }.map { ($0.offset % width, $0.offset / width) }
    }

    func count(of color: UInt32, frame f: Int? = nil) -> Int { pixels(frame: f).filter { $0 == color }.count }
    func visibleCount(frame f: Int? = nil) -> Int { pixels(frame: f).filter { $0 & 0xFF != 0 }.count }
}

/// Resets the shared model and every piece of global tool state, then starts a blank sprite.
@discardableResult
func fresh(_ w: Int = 16, _ h: Int = 16) -> SpriteModel {
    MainWindow.assumeKey = true
    let m = SpriteModel.shared
    m.stop()
    // One pristine untitled tab, whatever earlier scenario steps opened.
    let keep = m.activeSlotObject
    for s in m.slots where s !== keep {
        s.closed = true
        Autosaver.shared.discard(s)
        pixmith_free(s.doc)
    }
    m.slots = m.slots.filter { $0 === keep }
    m.activeSlot = 0
    m.replace(with: pixmith_new(UInt32(w), UInt32(h)))
    m.fileURL = nil
    keep?.untitledNumber = 1
    m.tool = .pencil
    m.color = BLACK
    m.brushSize = 1
    m.zoom = 8
    m.onionSkin = false
    m.showGrid = true
    m.loopTags = true
    m.opts.secondary = WHITE
    m.opts.filled = false
    m.opts.pixelPerfect = false
    m.opts.mirrorX = false
    m.opts.mirrorY = false
    m.opts.contiguous = true
    m.opts.ink = .simple
    m.opts.circleBrush = false
    m.opts.gradientRadial = false
    m.opts.gradientDither = 0
    m.opts.fillPattern = 0
    m.opts.sprayDensity = 40
    m.opts.shape = nil
    m.opts.drag = nil
    m.opts.textRequest = nil
    m.resetBrush()
    SelectionOptions.shared.tolerance = 0
    SelectionOptions.shared.contiguous = true
    TimelineSelection.shared.range = nil
    TimelineSelection.shared.focused = false
    let s = ViewSettings.shared
    s.tiled = .none
    s.onionPast = 1
    s.onionFuture = 0
    s.customGrid = false
    return m
}

let tmpDir: URL = {
    let u = FileManager.default.temporaryDirectory.appendingPathComponent("pixmith-qa-\(ProcessInfo.processInfo.processIdentifier)")
    try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
    return u
}()

func tmp(_ name: String) -> URL { tmpDir.appendingPathComponent(name) }

func pumpRunLoop(_ seconds: Double = 0.05) {
    RunLoop.main.run(until: Date(timeIntervalSinceNow: seconds))
}

/// Last core error, for messages.
var coreError: String { String(cString: pixmith_last_error()) }

/// Composite of one frame of a raw document handle, as 0xRRGGBBAA pixels.
func renderPixels(_ doc: OpaquePointer, frame: Int) -> [UInt32] {
    let w = Int(pixmith_width(doc)), h = Int(pixmith_height(doc))
    var buf = [UInt8](repeating: 0, count: w * h * 4)
    _ = pixmith_render(doc, UInt32(frame), &buf, buf.count)
    var out = [UInt32](repeating: 0, count: w * h)
    for i in 0..<w * h {
        let r = UInt32(buf[i * 4]), g = UInt32(buf[i * 4 + 1]), b = UInt32(buf[i * 4 + 2]), a = UInt32(buf[i * 4 + 3])
        out[i] = r << 24 | g << 16 | b << 8 | a
    }
    return out
}
