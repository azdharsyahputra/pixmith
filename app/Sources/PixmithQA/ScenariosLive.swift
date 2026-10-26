import AppKit
import Darwin
@testable import PixmithKit
import PixmithCore

// pixmith-qa scenarios for Pixmith Live (docs/LIVE_PROTOCOL.md): the real server on a loopback port, a plain BSD
// socket client that pumps the run loop while it waits (the server answers on the main thread), and the real model.

/// A client of the Live server. Non-blocking: waiting for an answer runs the main run loop, which is where the
/// requests are handled.
final class LiveClient {
    let fd: Int32
    private var pending: [UInt8] = []
    private var next = 1

    init?(port: Int) {
        fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = UInt16(port).bigEndian
        addr.sin_addr = in_addr(s_addr: UInt32(0x7F00_0001).bigEndian)
        let ok = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard ok == 0 else {
            close(fd)
            return nil
        }
        _ = fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK)
    }

    deinit { close(fd) }

    func send(_ text: String) {
        let bytes = Array((text + "\n").utf8)
        var offset = 0
        while offset < bytes.count {
            let n = Darwin.send(fd, bytes, bytes.count - offset, 0)
            if n > 0 { offset += n } else if errno != EAGAIN { return } else { pumpRunLoop(0.005) }
        }
    }

    /// The next line, or nil when the server closed the connection (or nothing came within `seconds`).
    func readLine(seconds: Double = 15) -> [String: Any]? {
        let deadline = Date(timeIntervalSinceNow: seconds)
        var chunk = [UInt8](repeating: 0, count: 1 << 16)
        while Date() < deadline {
            if let nl = pending.firstIndex(of: 0x0A) {
                let line = Array(pending[0..<nl])
                pending.removeFirst(nl + 1)
                return (try? JSONSerialization.jsonObject(with: Data(line))) as? [String: Any]
            }
            let n = recv(fd, &chunk, chunk.count, 0)
            if n > 0 {
                pending.append(contentsOf: chunk[0..<n])
            } else if n == 0 {
                return nil
            } else if errno == EAGAIN || errno == EINTR {
                pumpRunLoop(0.005)
            } else {
                return nil
            }
        }
        return nil
    }

    /// Sends a request without waiting for the answer (read it later with `readLine`).
    func post(_ method: String, _ params: [String: Any] = [:]) {
        let id = next
        next += 1
        let data = try! JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": id, "method": method, "params": params] as [String: Any])
        send(String(decoding: data, as: UTF8.self))
    }

    func call(_ method: String, _ params: [String: Any] = [:]) -> [String: Any] {
        post(method, params)
        return readLine() ?? ["error": ["code": 0, "message": "no answer"]]
    }

    /// `result` of a call that must succeed.
    func ok(_ method: String, _ params: [String: Any] = [:], file: String = #fileID, line: UInt = #line) -> [String: Any] {
        let r = call(method, params)
        expect(r["result"] != nil, "\(method) failed: \(r["error"] ?? "?")", file: file, line: line)
        return r["result"] as? [String: Any] ?? [:]
    }

    /// Error code of a call that must fail.
    func code(_ method: String, _ params: [String: Any] = [:], file: String = #fileID, line: UInt = #line) -> Int {
        let r = call(method, params)
        let code = (r["error"] as? [String: Any])?["code"] as? Int ?? 0
        expect(r["error"] != nil, "\(method) should have failed: \(r)", file: file, line: line)
        return code
    }

    /// True once the server has closed the connection.
    func closedByServer() -> Bool { readLine(seconds: 3) == nil && pending.isEmpty }
}

/// Starts the server in a clean state and connects a client that said hello.
func liveConnect() -> LiveClient? {
    let server = LiveServer.shared
    server.stop()
    guard server.start() else { return nil }
    guard let c = LiveClient(port: server.port) else { return nil }
    let r = c.call("hello", ["token": server.token, "client": "pixmith-qa", "client_version": "1", "protocol": 1])
    expect(r["result"] != nil, "hello failed: \(r)")
    return c
}

/// A document handle built the way a client builds one, as .pxmt bytes in base64: a `side` square with one pixel.
func liveDocument(side: Int, pixel: (Int, Int, UInt32)? = nil) -> String {
    let d = pixmith_new(UInt32(side), UInt32(side))!
    defer { pixmith_free(d) }
    if let (x, y, color) = pixel { pixmith_draw_line(d, 0, 0, Int32(x), Int32(y), Int32(x), Int32(y), color, 1) }
    var len = 0
    let bytes = pixmith_encode_document(d, &len)!
    defer { pixmith_bytes_free(bytes, len) }
    return Data(bytes: bytes, count: len).base64EncodedString()
}

private func docs(_ r: [String: Any]) -> [[String: Any]] { r["documents"] as? [[String: Any]] ?? [] }
private func doc(_ r: [String: Any]) -> [String: Any] { r["document"] as? [String: Any] ?? [:] }
private func rev(_ r: [String: Any]) -> Int { doc(r)["revision"] as? Int ?? -1 }

func registerLive() {
    scenario("live/hello-token-and-discovery") {
        let m = resetTabs()
        _ = m
        defer { LiveServer.shared.stop(); resetTabs() }
        let server = LiveServer.shared
        server.stop()
        expect(server.start())
        // The discovery file.
        let file = LiveServer.liveFolder.appendingPathComponent("\(ProcessInfo.processInfo.processIdentifier).json")
        expectEq(server.discoveryURL, file)
        let info = (try? Data(contentsOf: file)).flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } ?? [:]
        expectEq(info["protocol"] as? Int, 1)
        expectEq(info["pid"] as? Int, Int(ProcessInfo.processInfo.processIdentifier))
        expectEq(info["port"] as? Int, server.port)
        expectEq((info["token"] as? String)?.count, 32)
        expectEq(info["token"] as? String, server.token)
        expectEq(info["app"] as? String, "Pixmith")
        expectEq(info["front_end"] as? String, "swift")
        expectEq(info["platform"] as? String, "macos")
        expect((info["started"] as? Int ?? 0) > 1_700_000_000, "started")
        expect(info["version"] is String, "version")
        let perms = (try? FileManager.default.attributesOfItem(atPath: file.path))?[.posixPermissions] as? Int
        expectEq(perms, 0o600, "discovery file mode")
        expectEq(file.deletingLastPathComponent().path, AppSupport.directory.appendingPathComponent("Pixmith/Live").path)

        // Anything before hello, a wrong token and garbage close the connection.
        let early = LiveClient(port: server.port)!
        expectEq(early.code("documents.list"), -32001)
        expect(early.closedByServer(), "closed after a request before hello")
        let wrong = LiveClient(port: server.port)!
        expectEq((wrong.call("hello", ["token": "nope", "protocol": 1])["error"] as? [String: Any])?["code"] as? Int, -32001)
        expect(wrong.closedByServer(), "closed after a wrong token")
        let junk = LiveClient(port: server.port)!
        junk.send("this is not json")
        expectEq((junk.readLine()?["error"] as? [String: Any])?["code"] as? Int, -32700)
        expect(junk.closedByServer())

        // The real hello.
        let c = LiveClient(port: server.port)!
        let r = c.call("hello", ["token": server.token, "client": "pixmith-mcp", "client_version": "9", "protocol": 1])
        let result = r["result"] as? [String: Any] ?? [:]
        expectEq(result["app"] as? String, "Pixmith")
        expectEq(result["front_end"] as? String, "swift")
        expectEq(result["protocol"] as? Int, 1)
        expectEq(result["platform"] as? String, "macos")
        pumpRunLoop(0.1)
        expectEq(server.clients.map(\.name), ["pixmith-mcp"])
        expectEq(c.code("no.such.method"), -32601)
        expectEq(c.code("documents.get"), -32602)
        expectEq(c.code("documents.get", ["id": "doc-99"]), -32004)
        c.send("{oops")
        expectEq((c.readLine()?["error"] as? [String: Any])?["code"] as? Int, -32700)
        c.send("[1,2]")
        expectEq((c.readLine()?["error"] as? [String: Any])?["code"] as? Int, -32600)
        // Answers come in order.
        c.send("{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"method\":\"documents.list\",\"params\":{}}")
        c.send("{\"jsonrpc\":\"2.0\",\"id\":\"b\",\"method\":\"view.get\",\"params\":{}}")
        expectEq(c.readLine()?["id"] as? String, "a")
        expectEq(c.readLine()?["id"] as? String, "b")

        // Stopping closes the port and removes the file.
        server.stop()
        expect(!FileManager.default.fileExists(atPath: file.path), "discovery file removed")
        expect(LiveClient(port: server.port) == nil, "port closed")
    }

    scenario("live/list-get-view") {
        let m = resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        let list = c.ok("documents.list")
        expectEq(docs(list).count, 1)
        let d = docs(list)[0]
        expectEq(d["id"] as? String, "doc-1")
        expectEq(list["active"] as? String, "doc-1")
        expectEq(d["width"] as? Int, m.width)
        expectEq(d["frames"] as? Int, 1)
        expectEq(d["layers"] as? Int, 1)
        expectEq(d["revision"] as? Int, 1)
        expect(d["path"] is NSNull, "no path yet")
        expectEq(d["dirty"] as? Bool, false)
        expectEq(d["active"] as? Bool, true)
        expectEq(d["name"] as? String, "Untitled")
        let got = c.ok("documents.get", ["id": "doc-1"])
        let bytes = Data(base64Encoded: got["pxmt"] as? String ?? "") ?? Data()
        let file = tmp("live-get.pxmt")
        try? bytes.write(to: file)
        let copy = file.path.withCString { pixmith_open($0) }
        expect(copy != nil, "the bytes open as a document")
        if let copy {
            expectEq(Int(pixmith_width(copy)), m.width)
            pixmith_free(copy)
        }
        expect(got["selection"] is NSNull, "no selection")
        let v = c.ok("view.get")
        expectEq(v["document"] as? String, "doc-1")
        expectEq(v["tool"] as? String, "pencil")
        expectEq(v["zoom"] as? Double, Double(m.zoom))
        expectEq(v["layer"] as? Int, m.layer)
        m.tool = .shapePolygon
        expectEq(c.ok("view.get")["tool"] as? String, "shape_polygon")
        m.tool = .pencil
    }

    scenario("live/apply-is-one-labelled-undo-step") {
        let m = resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        let side = m.width
        let before = m.pixels()
        let r = c.ok("documents.apply", ["id": "doc-1", "base_revision": 1, "label": "Plot a pixel",
                                         "pxmt": liveDocument(side: side, pixel: (3, 4, RED))])
        expectEq(rev(r), 2)
        expectEq(doc(r)["dirty"] as? Bool, true)
        expectPixel(3, 4, RED)
        expectEq(m.historyCounts.undo, 1)
        let label = pixmith_history_label(m.doc, 0).map { p -> String in defer { pixmith_string_free(p) }; return String(cString: p) }
        expectEq(label, "AI: Plot a pixel")
        // The user's own Undo takes it back, and Redo returns it.
        m.undo()
        expectEq(m.pixels(), before)
        m.redo()
        expectPixel(3, 4, RED)
        // documents.undo / redo walk the same history and bump the revision.
        let revBefore = rev(c.ok("documents.get", ["id": "doc-1"]))
        let undone = c.ok("documents.undo", ["id": "doc-1"])
        expectEq(undone["steps"] as? Int, 1)
        expectEq(m.pixels(), before)
        expect(rev(undone) > revBefore, "undo bumps the revision")
        expectEq(c.ok("documents.undo", ["id": "doc-1", "steps": 5])["steps"] as? Int, 0)
        let redone = c.ok("documents.redo", ["id": "doc-1", "steps": 3])
        expectEq(redone["steps"] as? Int, 1)
        expectPixel(3, 4, RED)
        expectEq(c.code("documents.undo", ["id": "doc-1", "steps": -1]), -32602)
        // Another size changes the canvas.
        let grown = c.ok("documents.apply", ["id": "doc-1", "base_revision": rev(redone), "label": "Grow",
                                             "pxmt": liveDocument(side: side * 2)])
        expectEq(doc(grown)["width"] as? Int, side * 2)
        expectEq(m.width, side * 2)
        m.undo()
        expectEq(m.width, side)
    }

    scenario("live/stale-revision-and-bad-bytes") {
        let m = resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        let side = m.width
        let before = m.pixels()
        let good = liveDocument(side: side, pixel: (1, 1, BLUE))
        expectEq(c.code("documents.apply", ["id": "doc-1", "base_revision": 7, "label": "x", "pxmt": good]), -32009)
        expectEq(m.pixels(), before)
        // The user draws: the revision goes up without the client doing anything.
        m.line(from: (0, 0), to: (0, 0), erase: false)
        m.changed()
        let current = rev(c.ok("documents.get", ["id": "doc-1"]))
        expect(current > 1, "drawing bumps the revision")
        expectEq(c.code("documents.apply", ["id": "doc-1", "base_revision": 1, "label": "x", "pxmt": good]), -32009)
        let steps = m.historyCounts
        expectEq(c.code("documents.apply", ["id": "doc-1", "base_revision": current, "pxmt": "@@@"]), -32602)
        expectEq(c.code("documents.apply", ["id": "doc-1", "base_revision": current, "pxmt": "aGVsbG8="]), -32000)
        expectEq(c.code("documents.apply", ["id": "doc-1", "pxmt": good]), -32602)
        expectEq(c.code("documents.apply", ["id": "doc-4", "base_revision": current, "pxmt": good]), -32004)
        expect(m.historyCounts == steps, "failed applies take no undo step")
        // Layer and frame are clamped.
        let r = c.ok("documents.apply", ["id": "doc-1", "base_revision": current, "label": "y", "layer": 99, "frame": -3, "pxmt": good])
        expectEq(doc(r)["layer"] as? Int, 0)
        expectEq(doc(r)["frame"] as? Int, 0)
    }

    scenario("live/new-open-save-activate-close") {
        let m = resetTabs()
        let folder = tmp("live-files")
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        // New reuses the blank tab; after an edit it opens another one.
        let first = doc(c.ok("documents.new", ["width": 12, "height": 9, "name": "hero"]))
        expectEq(first["width"] as? Int, 12)
        expectEq(first["name"] as? String, "hero")
        expectEq(m.slots.count, 1)
        let id1 = first["id"] as? String ?? ""
        let edited = c.ok("documents.apply", ["id": id1, "base_revision": first["revision"] as? Int ?? 0, "label": "art",
                                              "pxmt": liveDocument(side: 12, pixel: (1, 1, RED))])
        expectEq(doc(edited)["dirty"] as? Bool, true)
        let second = doc(c.ok("documents.new", ["width": 8, "height": 8]))
        expectEq(m.slots.count, 2)
        expect((second["id"] as? String) != id1, "a new id")
        expectEq(second["active"] as? Bool, true)
        expectEq(c.code("documents.new", ["width": 0, "height": 8]), -32602)
        expectEq(c.code("documents.new", ["width": 9999, "height": 8]), -32602)
        let list = c.ok("documents.list")
        expectEq(docs(list).count, 2)
        expectEq(list["active"] as? String, second["id"] as? String)
        // Activate the first.
        expectEq(doc(c.ok("documents.activate", ["id": id1]))["active"] as? Bool, true)
        expectEq(m.activeSlot, 0)
        // Save: needs a path for a new document, and an absolute one.
        expectEq(c.code("documents.save", ["id": id1]), -32602)
        expectEq(c.code("documents.save", ["id": id1, "path": "relative.pxmt"]), -32602)
        expectEq(c.code("documents.save", ["id": id1, "path": folder.appendingPathComponent("x.png").path]), -32602)
        let out = folder.appendingPathComponent("hero.pxmt")
        let saved = doc(c.ok("documents.save", ["id": id1, "path": out.path]))
        expectEq(saved["name"] as? String, "hero.pxmt")
        expectEq(saved["path"] as? String, out.path)
        expectEq(saved["dirty"] as? Bool, false)
        expect(((try? Data(contentsOf: out)) ?? Data()).starts(with: Array("PXMT".utf8)) || FileManager.default.fileExists(atPath: out.path), "file written")
        _ = c.ok("documents.save", ["id": id1])
        let ase = folder.appendingPathComponent("hero.aseprite")
        _ = c.ok("documents.save", ["id": id1, "path": ase.path])
        expect(FileManager.default.fileExists(atPath: ase.path), "aseprite written")
        // Saving the non-active tab works too.
        let other = doc(c.ok("documents.save", ["id": second["id"] as? String ?? "", "path": folder.appendingPathComponent("second.pxmt").path]))
        expectEq(other["dirty"] as? Bool, false)
        expectEq(m.activeSlot, 0, "saving another tab does not switch to it")
        // Open: a file, and the tab that already has it.
        let file = folder.appendingPathComponent("other.pxmt")
        let handle = pixmith_new(20, 10)!
        _ = file.path.withCString { pixmith_save(handle, $0) }
        pixmith_free(handle)
        let opened = doc(c.ok("documents.open", ["path": file.path]))
        expectEq(opened["width"] as? Int, 20)
        expectEq(opened["name"] as? String, "other.pxmt")
        let tabs = m.slots.count
        expectEq(doc(c.ok("documents.open", ["path": file.path]))["id"] as? String, opened["id"] as? String)
        expectEq(m.slots.count, tabs)
        expectEq(c.code("documents.open", ["path": folder.appendingPathComponent("missing.pxmt").path]), -32000)
        expectEq(c.code("documents.open", ["path": folder.path]), -32602)
        // Close: unsaved changes need `discard`.
        let oid = opened["id"] as? String ?? ""
        let dirty = c.ok("documents.apply", ["id": oid, "base_revision": opened["revision"] as? Int ?? 0, "label": "z",
                                             "pxmt": liveDocument(side: 20, pixel: (0, 0, RED))])
        expectEq(doc(dirty)["dirty"] as? Bool, true)
        SpriteModel.alertRunner = { _ in AlertScript.cancel }
        expectEq(c.code("documents.close", ["id": oid]), -32011)
        expectEq(c.code("documents.close", ["id": oid, "discard": false]), -32011)
        let count = m.slots.count
        _ = c.ok("documents.close", ["id": oid, "discard": true])
        expectEq(m.slots.count, count - 1)
        expectEq(c.code("documents.get", ["id": oid]), -32004)
        _ = c.ok("documents.close", ["id": id1])
        expectEq(m.slots.count, count - 2)
    }

    scenario("live/parallel-documents-new-keep-both") {
        let m = resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        expectEq(m.slots.count, 1)
        // Two requests back to back, before either answer is read: the second must not reuse the first's tab.
        c.post("documents.new", ["width": 10, "height": 6, "name": "one"])
        c.post("documents.new", ["width": 7, "height": 5, "name": "two"])
        let a = doc((c.readLine() ?? [:])["result"] as? [String: Any] ?? [:])
        let b = doc((c.readLine() ?? [:])["result"] as? [String: Any] ?? [:])
        expect((a["id"] as? String) != nil && (a["id"] as? String) != (b["id"] as? String), "two different documents")
        expectEq(a["width"] as? Int, 10)
        expectEq(b["width"] as? Int, 7)
        expectEq(m.slots.count, 2, "both documents stay open (the app's blank tab was reused once)")
        let list = docs(c.ok("documents.list"))
        expectEq(list.compactMap { $0["id"] as? String }.sorted(), [a["id"] as? String ?? "", b["id"] as? String ?? ""].sorted())
        expectEq(Set(list.compactMap { $0["name"] as? String }), ["one", "two"])
        expectEq(list.first { $0["id"] as? String == b["id"] as? String }?["active"] as? Bool, true)
        // Still never reused, even though both are blank and never edited.
        let third = doc(c.ok("documents.new", ["width": 4, "height": 4]))
        expectEq(m.slots.count, 3)
        expect((third["id"] as? String) != (b["id"] as? String), "third is its own document")
    }

    scenario("live/view-set") {
        let m = resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        m.addLayer()
        m.addFrame(duplicate: false)
        guard let c = liveConnect() else { return expect(false, "no server") }
        let v = c.ok("view.set", ["layer": 0, "frame": 1])
        expectEq(v["layer"] as? Int, 0)
        expectEq(v["frame"] as? Int, 1)
        expectEq(m.layer, 0)
        expectEq(m.frame, 1)
        let w = c.ok("view.set", ["id": "doc-1", "layer": 50])
        expectEq(w["layer"] as? Int, 1)
        expectEq(w["document"] as? String, "doc-1")
        expectEq(c.code("view.set", ["id": "doc-9", "layer": 1]), -32004)
        expectEq(c.code("view.set", ["layer": "one"]), -32602)
        let got = c.ok("documents.list")
        expectEq(docs(got)[0]["layer"] as? Int, 1)
        expectEq(docs(got)[0]["frame"] as? Int, 1)
    }

    scenario("live/busy-while-a-sheet-is-open") {
        let m = resetTabs()
        defer { m.showNewSheet = false; LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        m.showNewSheet = true
        _ = c.ok("documents.list")
        _ = c.ok("documents.get", ["id": "doc-1"])
        let params: [String: Any] = ["id": "doc-1", "base_revision": 1, "label": "x", "pxmt": liveDocument(side: m.width)]
        expectEq(c.code("documents.apply", params), -32010)
        expectEq(c.code("documents.new", ["width": 4, "height": 4]), -32010)
        expectEq(c.code("documents.get", ["id": "nope"]), -32004)
        expectEq(m.historyCounts.undo, 0)
        m.showNewSheet = false
        _ = c.ok("documents.apply", params)
    }

    scenario("live/godot-atlas") {
        let m = resetTabs()
        defer { m.showNewSheet = false; LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        m.addFrame(duplicate: false)
        expectEq(Int(pixmith_frame_count(m.doc)), 2)
        let r = c.ok("godot.atlas", ["id": "doc-1"])
        expectEq(doc(r)["id"] as? String, "doc-1")
        expectEq(doc(r)["frames"] as? Int, 2)
        expectEq(r["linked"] as? Bool, false)
        let resources = r["resources"] as? [[String: Any]] ?? []
        expectEq(resources.count, 1)
        let pages = resources.first?["pages"] as? [[String: Any]] ?? []
        expect(!pages.isEmpty, "an atlas page")
        let png = Data(base64Encoded: pages.first?["png"] as? String ?? "") ?? Data()
        expectEq(Array(png.prefix(8)), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
        expect(NSImage(data: png) != nil, "the page decodes as a PNG")
        let animations = resources.first?["animations"] as? [[String: Any]] ?? []
        let frames = animations.flatMap { $0["frames"] as? [[String: Any]] ?? [] }
        expectEq(frames.count, 2)
        for f in frames { expectEq((f["region"] as? [Any])?.count, 4) }
        expectEq(c.code("godot.atlas", ["id": "doc-99"]), -32004)
        expectEq(c.code("godot.atlas"), -32602)
        // Read-only: answered while a sheet is open, and an unknown id is still reported.
        m.showNewSheet = true
        expect(c.ok("godot.atlas", ["id": "doc-1"])["resources"] is [Any], "answered while busy")
        expectEq(c.code("godot.atlas", ["id": "nope"]), -32004)
    }

    scenario("live/selection-sync") {
        let m = resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        let side = m.width
        pixmith_sel_rect(m.doc, 3, 2, 5, 3)
        m.changed()
        let got = c.ok("documents.get", ["id": "doc-1"])
        let sel = got["selection"] as? [String: Any] ?? [:]
        expectEq([sel["x"], sel["y"], sel["w"], sel["h"]].map { $0 as? Int ?? -1 }, [3, 2, 3, 2])
        expectEq([UInt8](Data(base64Encoded: sel["mask"] as? String ?? "") ?? Data()), [UInt8](repeating: 1, count: 6))
        var r = c.ok("documents.apply", ["id": "doc-1", "base_revision": rev(got), "label": "keep", "pxmt": liveDocument(side: side)])
        expect(m.selected(3, 2), "the selection is kept")
        let mask = Data([1, 0, 1, 1]).base64EncodedString()
        let steps = m.historyCounts.undo
        r = c.ok("documents.apply", ["id": "doc-1", "base_revision": rev(r), "label": "set", "pxmt": liveDocument(side: side),
                                     "selection": ["x": 1, "y": 1, "w": 2, "h": 2, "mask": mask] as [String: Any]])
        expectEq(m.historyCounts.undo, steps + 1, "one undo step")
        expect(m.selected(1, 1) && !m.selected(2, 1) && m.selected(1, 2) && m.selected(2, 2), "the new selection")
        expect(!m.selected(3, 2), "the old one is gone")
        expectEq(c.code("documents.apply", ["id": "doc-1", "base_revision": rev(r), "pxmt": liveDocument(side: side),
                                            "selection": ["x": 0, "y": 0, "w": 2, "h": 2, "mask": "AQ=="] as [String: Any]]), -32602)
        r = c.ok("documents.apply", ["id": "doc-1", "base_revision": rev(r), "label": "clear", "pxmt": liveDocument(side: side),
                                     "selection": NSNull()])
        expect(c.ok("documents.get", ["id": "doc-1"])["selection"] is NSNull, "cleared")
        m.undo()
        expect(m.selected(1, 1), "undo brings the previous selection back")
    }

    scenario("live/game-capture") {
        resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        let server = LiveServer.shared
        server.stop()
        guard server.start() else { return expect(false, "no server") }
        func connect(_ name: String) -> LiveClient? {
            guard let c = LiveClient(port: server.port) else { return nil }
            let r = c.call("hello", ["token": server.token, "client": name, "client_version": "1", "protocol": 1])
            expect(r["result"] != nil, "hello failed: \(r)")
            return c
        }
        guard let game = connect("pixmith-godot"), let mcp = connect("pixmith-mcp") else { return expect(false, "no client") }
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
        func errorCode(_ r: [String: Any]?) -> Int? { (r?["error"] as? [String: Any])?["code"] as? Int }
        /// Polls as the game until it is handed a request id (the capture is posted on another connection).
        func nextRequest() -> String? {
            for _ in 0..<150 {
                if let id = game.ok("game.poll")["capture"] as? String { return id }
                pumpRunLoop(0.02)
            }
            return nil
        }

        // Like every method, the game methods need a hello first.
        let early = LiveClient(port: server.port)!
        expectEq(early.code("game.poll"), -32001)

        // Nobody has polled: no running game, and the answer is quick.
        var started = Date()
        expectEq(mcp.code("game.capture"), -32012)
        expect(Date().timeIntervalSince(started) < 1, "no poller fails at once")

        // An idle poll returns null; a bad timeout is a parameter error.
        expect(game.ok("game.poll")["capture"] is NSNull, "nothing asked yet")
        expectEq(mcp.code("game.capture", ["timeout_ms": "soon"]), -32602)

        // The whole relay: B asks, A's next poll gets an id, A answers, B gets the frame with A's client name.
        mcp.post("game.capture", ["timeout_ms": 5000])
        guard let request = nextRequest() else { return expect(false, "the game never got a request id") }
        expect(game.ok("game.poll")["capture"] is NSNull, "a request id is handed out once")
        expectEq(game.code("game.frame", ["request": request, "png": png, "w": "wide", "h": 1]), -32602)
        let answer = game.ok("game.frame", ["request": request, "png": png, "w": 1, "h": 1])
        expect(answer.isEmpty, "frame is acknowledged with {}")
        let frame = mcp.readLine()?["result"] as? [String: Any] ?? [:]
        expectEq(frame["png"] as? String, png)
        expectEq(frame["w"] as? Int, 1)
        expectEq(frame["h"] as? Int, 1)
        expectEq(frame["client"] as? String, "pixmith-godot")

        // A second frame for an answered request, and one for an unknown request, are dropped with {}.
        expect(game.ok("game.frame", ["request": request, "png": png, "w": 1, "h": 1]).isEmpty, "duplicate frame")
        expect(game.ok("game.frame", ["request": "cap-999", "png": png, "w": 1, "h": 1]).isEmpty, "unknown frame")

        // The poller never answers: -32012 after the timeout (and not much before it).
        started = Date()
        mcp.post("game.capture", ["timeout_ms": 400])
        guard let silent = nextRequest() else { return expect(false, "no request id for the silent game") }
        let timedOut = mcp.readLine()
        let waited = Date().timeIntervalSince(started)
        expectEq(errorCode(timedOut), -32012)
        expect(waited >= 0.35 && waited < 3, "timed out after \(waited) s")
        // The late frame is stale now.
        expect(game.ok("game.frame", ["request": silent, "png": png, "w": 1, "h": 1]).isEmpty, "late frame dropped")

        // The game that polled last is the one asked.
        guard let second = connect("pixmith-godot-2") else { return expect(false, "no second game") }
        _ = second.ok("game.poll")
        mcp.post("game.capture", ["timeout_ms": 5000])
        var asked: String?
        for _ in 0..<150 where asked == nil {
            asked = second.ok("game.poll")["capture"] as? String
            if asked == nil { pumpRunLoop(0.02) }
        }
        expect(asked != nil, "the last poller was asked")
        expect(game.ok("game.poll")["capture"] is NSNull, "the earlier game was not asked")
        _ = second.ok("game.frame", ["request": asked ?? "", "png": png, "w": 2, "h": 3])
        let last = mcp.readLine()?["result"] as? [String: Any] ?? [:]
        expectEq(last["client"] as? String, "pixmith-godot-2")
        expectEq(last["w"] as? Int, 2)

        // A game that stopped polling is no longer running: after a second, capture fails at once.
        pumpRunLoop(1.2)
        started = Date()
        expectEq(mcp.code("game.capture", ["timeout_ms": 5000]), -32012)
        expect(Date().timeIntervalSince(started) < 1, "stale poller fails at once")
    }

    scenario("live/disconnect-by-the-user") {
        resetTabs()
        defer { LiveServer.shared.stop(); resetTabs() }
        guard let c = liveConnect() else { return expect(false, "no server") }
        let server = LiveServer.shared
        _ = c.ok("documents.list")
        pumpRunLoop(0.1)
        expectEq(server.clients.count, 1)
        server.disconnectByUser()
        expect(c.closedByServer(), "connection closed")
        pumpRunLoop(0.1)
        expectEq(server.clients.count, 0)
        // A client that comes back with the right token is turned away.
        let again = LiveClient(port: server.port)!
        let r = again.call("hello", ["token": server.token, "protocol": 1])
        let e = r["error"] as? [String: Any]
        expectEq(e?["code"] as? Int, -32001)
        expectEq(e?["message"] as? String, "disconnected by the user")
        expect(again.closedByServer())
        // Switching Live mode off and on again lets clients in.
        server.stop()
        guard let fresh = liveConnect() else { return expect(false, "no server after restart") }
        _ = fresh.ok("documents.list")
    }

    scenario("live/preference-follows-the-switch") {
        resetTabs()
        let key = Prefs.Key.liveEnabled
        let saved = UserDefaults.standard.object(forKey: key)
        defer {
            if let saved { UserDefaults.standard.set(saved, forKey: key) } else { UserDefaults.standard.removeObject(forKey: key) }
            LiveServer.shared.stop()
            resetTabs()
        }
        UserDefaults.standard.removeObject(forKey: key)
        expect(Prefs.liveEnabled, "on by default")
        LiveServer.shared.applyPreference()
        expect(LiveServer.shared.isRunning, "started")
        let file = LiveServer.shared.discoveryURL
        expect(file.map { FileManager.default.fileExists(atPath: $0.path) } ?? false, "discovery file exists")
        UserDefaults.standard.set(false, forKey: key)
        LiveServer.shared.applyPreference()
        expect(!LiveServer.shared.isRunning, "stopped")
        expect(!(file.map { FileManager.default.fileExists(atPath: $0.path) } ?? true), "discovery file removed")
    }
}
