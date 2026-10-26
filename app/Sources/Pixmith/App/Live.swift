import AppKit
import Darwin
import Foundation
import PixmithCore
import SwiftUI

// Pixmith Live (docs/LIVE_PROTOCOL.md): lets a local tool such as the Pixmith MCP server read and edit the documents
// open in the app. `LiveServer` runs the loopback TCP server on background threads; every request is handed to the
// main thread, where `LiveHandler` answers it, so all document access happens there.

/// What a tab looked like when its Live revision was last updated: a difference means the document changed.
struct LiveStamp: Equatable {
    var revision: Int
    var state: UInt64
}

struct LiveError: Error {
    let code: Int
    let message: String

    static func params(_ message: String) -> LiveError { LiveError(code: -32602, message: message) }
    static func other(_ message: String) -> LiveError { LiveError(code: -32000, message: message) }
    static func busy(_ message: String) -> LiveError { LiveError(code: -32010, message: message) }
    static func noGame(_ message: String) -> LiveError { LiveError(code: -32012, message: message) }
    static let unauthorized = LiveError(code: -32001, message: "send hello with the token first")
}

typealias LiveReply = Result<[String: Any], LiveError>

// MARK: - Server

final class LiveServer: ObservableObject {
    static let shared = LiveServer()

    struct Client: Identifiable, Equatable {
        let id: Int32
        var name: String
        var version: String
    }

    /// Clients that completed `hello` and are still connected (the "AI connected" chip shows while there is one).
    @Published private(set) var clients: [Client] = []
    @Published private(set) var isRunning = false
    @Published private(set) var startError: String?
    private(set) var port = 0
    private(set) var token = ""
    private(set) var discoveryURL: URL?

    static let protocolVersion = 1
    static let maxMessage = 64 << 20
    private static let helloTimeout = 10
    private static let answerTimeout = 120.0

    private var session: Session?

    /// Everything one running server owns; a restart makes a new one, so old threads cannot touch the new server.
    private final class Session {
        let token: String
        var listenFD: Int32
        let lock = NSLock()
        var stopped = false
        /// The user pressed Disconnect: `hello` is refused for the rest of this session.
        var blocked = false
        var fds = Set<Int32>()
        let acceptDone = DispatchSemaphore(value: 0)
        /// The Game view relay (`game.*`); it lives and dies with the session.
        let game = GameRelay()

        init(token: String, listenFD: Int32) {
            self.token = token
            self.listenFD = listenFD
        }

        var isStopped: Bool { lock.lock(); defer { lock.unlock() }; return stopped }
        var isBlocked: Bool { lock.lock(); defer { lock.unlock() }; return blocked }
    }

    private init() {}

    /// Starts or stops the server to match the preference.
    func applyPreference() {
        if Prefs.liveEnabled { start() } else { stop() }
    }

    // MARK: Start and stop

    @discardableResult
    func start() -> Bool {
        guard session == nil else { return true }
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return fail("Live mode could not open a socket.") }
        var yes: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = 0
        addr.sin_addr = in_addr(s_addr: UInt32(0x7F00_0001).bigEndian)
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard bound == 0, listen(fd, 8) == 0 else {
            close(fd)
            return fail("Live mode could not listen on the loopback address.")
        }
        var named = sockaddr_in()
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        _ = withUnsafeMutablePointer(to: &named) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &length) }
        }
        let token = Self.randomToken()
        port = Int(UInt16(bigEndian: named.sin_port))
        self.token = token
        guard let url = writeDiscovery(port: port, token: token) else {
            close(fd)
            return fail("Live mode could not write its discovery file in Application Support.")
        }
        discoveryURL = url
        let session = Session(token: token, listenFD: fd)
        self.session = session
        isRunning = true
        startError = nil
        Thread.detachNewThread { [weak self] in self?.accept(session) }
        return true
    }

    private func fail(_ message: String) -> Bool {
        startError = message
        return false
    }

    /// Closes the port and every connection and removes the discovery file.
    func stop() {
        guard let session else { return }
        self.session = nil
        session.lock.lock()
        session.stopped = true
        for fd in session.fds { shutdown(fd, SHUT_RDWR) }
        session.lock.unlock()
        // The accept loop notices within a poll interval and closes the listening socket.
        _ = session.acceptDone.wait(timeout: .now() + 2)
        if let url = discoveryURL { try? FileManager.default.removeItem(at: url) }
        discoveryURL = nil
        clients = []
        isRunning = false
    }

    /// Closes every connection; a client may connect again.
    func disconnectAll() {
        guard let session else { return }
        session.lock.lock()
        for fd in session.fds { shutdown(fd, SHUT_RDWR) }
        session.lock.unlock()
    }

    /// The Disconnect button: closes every connection and refuses later `hello`s ("disconnected by the user") until
    /// Live mode is turned off and on again, which starts a new server with a new port and token.
    func disconnectByUser() {
        session?.lock.lock()
        session?.blocked = true
        session?.lock.unlock()
        disconnectAll()
    }

    // MARK: Discovery

    static var liveFolder: URL { AppSupport.directory.appendingPathComponent("Pixmith/Live", isDirectory: true) }

    private static func randomToken() -> String {
        var bytes = [UInt8](repeating: 0, count: 16)
        arc4random_buf(&bytes, bytes.count)
        return bytes.map { String(format: "%02x", $0) }.joined()
    }

    private func writeDiscovery(port: Int, token: String) -> URL? {
        let folder = Self.liveFolder
        do { try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true) } catch { return nil }
        removeStaleDiscoveryFiles(in: folder)
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.4.0"
        let pid = ProcessInfo.processInfo.processIdentifier
        let text = "{\"protocol\": \(Self.protocolVersion), \"pid\": \(pid), \"port\": \(port), \"token\": \"\(token)\", "
            + "\"app\": \"Pixmith\", \"version\": \"\(version)\", \"front_end\": \"swift\", \"platform\": \"macos\", "
            + "\"started\": \(Int(Date().timeIntervalSince1970))}"
        let url = folder.appendingPathComponent("\(pid).json")
        let tmp = folder.appendingPathComponent("\(pid).json.tmp")
        try? FileManager.default.removeItem(at: tmp)
        guard FileManager.default.createFile(atPath: tmp.path, contents: Data(text.utf8), attributes: [.posixPermissions: 0o600]),
              rename(tmp.path, url.path) == 0 else { return nil }
        return url
    }

    /// Deletes discovery files of instances that are gone (their port refuses connections).
    private func removeStaleDiscoveryFiles(in folder: URL) {
        let files = (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)) ?? []
        for file in files where file.pathExtension == "json" {
            let port = (try? Data(contentsOf: file))
                .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
                .flatMap { $0["port"] as? Int }
            if port.map({ !Self.accepts(port: $0) }) ?? true { try? FileManager.default.removeItem(at: file) }
        }
    }

    private static func accepts(port: Int) -> Bool {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return false }
        defer { close(fd) }
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = UInt16(truncatingIfNeeded: port).bigEndian
        addr.sin_addr = in_addr(s_addr: UInt32(0x7F00_0001).bigEndian)
        return withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        } == 0
    }

    // MARK: Connections

    private func accept(_ session: Session) {
        defer {
            close(session.listenFD)
            session.acceptDone.signal()
        }
        while !session.isStopped {
            var pfd = pollfd(fd: session.listenFD, events: Int16(POLLIN), revents: 0)
            guard poll(&pfd, 1, 200) > 0, pfd.revents & Int16(POLLIN) != 0 else { continue }
            let fd = Darwin.accept(session.listenFD, nil, nil)
            guard fd >= 0 else { continue }
            session.lock.lock()
            let full = session.fds.count >= 16 || session.stopped
            if !full { session.fds.insert(fd) }
            session.lock.unlock()
            if full {
                close(fd)
                continue
            }
            var yes: Int32 = 1
            setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size))
            Thread.detachNewThread { [weak self] in
                self?.serve(fd, session)
                session.lock.lock()
                session.fds.remove(fd)
                session.lock.unlock()
                session.game.forget(fd)
                close(fd)
                DispatchQueue.main.async { self?.clients.removeAll { $0.id == fd } }
            }
        }
    }

    private func setReceiveTimeout(_ fd: Int32, seconds: Int) {
        var tv = timeval(tv_sec: seconds, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
    }

    private func send(_ fd: Int32, id: Any, reply: LiveReply) -> Bool {
        var object: [String: Any] = ["jsonrpc": "2.0", "id": id]
        switch reply {
        case .success(let result): object["result"] = result
        case .failure(let e): object["error"] = ["code": e.code, "message": e.message] as [String: Any]
        }
        if !JSONSerialization.isValidJSONObject(object) {
            object["result"] = nil
            object["error"] = ["code": -32000, "message": "internal error: the answer is not valid JSON"] as [String: Any]
        }
        guard var data = try? JSONSerialization.data(withJSONObject: object, options: [.withoutEscapingSlashes]) else { return false }
        data.append(0x0A)
        return data.withUnsafeBytes { raw -> Bool in
            var offset = 0
            while offset < raw.count {
                let n = Darwin.send(fd, raw.baseAddress!.advanced(by: offset), raw.count - offset, 0)
                if n < 0 {
                    if errno == EINTR { continue }
                    return false
                }
                offset += n
            }
            return true
        }
    }

    private enum Line {
        case message([UInt8])
        case closed
        case tooLong
    }

    /// Reads up to the next newline. `buffer` keeps what follows it.
    private func readLine(_ fd: Int32, _ buffer: inout [UInt8], _ scanned: inout Int) -> Line {
        var chunk = [UInt8](repeating: 0, count: 1 << 18)
        while true {
            if let nl = buffer[scanned...].firstIndex(of: 0x0A) {
                let line = Array(buffer[0..<nl])
                buffer.removeFirst(nl + 1)
                scanned = 0
                return line.count > Self.maxMessage ? .tooLong : .message(line)
            }
            scanned = buffer.count
            if buffer.count > Self.maxMessage { return .tooLong }
            let n = recv(fd, &chunk, chunk.count, 0)
            if n > 0 {
                buffer.append(contentsOf: chunk[0..<n])
            } else if n == 0 || errno != EINTR {
                return .closed
            }
        }
    }

    private func serve(_ fd: Int32, _ session: Session) {
        setReceiveTimeout(fd, seconds: Self.helloTimeout)
        var buffer: [UInt8] = []
        var scanned = 0
        var authed = false
        var clientName = ""
        while !session.isStopped {
            let bytes: [UInt8]
            switch readLine(fd, &buffer, &scanned) {
            case .closed: return
            case .tooLong:
                _ = send(fd, id: NSNull(), reply: .failure(LiveError(code: -32600, message: "message larger than 64 MiB")))
                return
            case .message(let b): bytes = b
            }
            if bytes.allSatisfy({ $0 == 0x20 || $0 == 0x09 || $0 == 0x0D }) { continue }
            guard let parsed = try? JSONSerialization.jsonObject(with: Data(bytes)) else {
                _ = send(fd, id: NSNull(), reply: .failure(LiveError(code: -32700, message: "invalid JSON")))
                if authed { continue }
                return
            }
            guard let message = parsed as? [String: Any], let method = message["method"] as? String else {
                _ = send(fd, id: (parsed as? [String: Any])?["id"] ?? NSNull(),
                         reply: .failure(LiveError(code: -32600, message: "not a JSON-RPC request")))
                if authed { continue }
                return
            }
            // Notifications are not used in protocol 1.
            guard let id = message["id"] else {
                if authed { continue }
                return
            }
            let params = message["params"] as? [String: Any] ?? [:]
            if !authed || method == "hello" {
                if let error = hello(method == "hello" ? params : nil, session) {
                    _ = send(fd, id: id, reply: .failure(error))
                    if !authed { return }
                    continue
                }
                if !authed {
                    authed = true
                    setReceiveTimeout(fd, seconds: 0)
                    let client = Client(id: fd, name: Self.clip(params["client"]), version: Self.clip(params["client_version"]))
                    clientName = client.name
                    DispatchQueue.main.async { [weak self] in self?.clients.append(client) }
                }
                if !send(fd, id: id, reply: .success(Self.helloResult())) { return }
                continue
            }
            // The Game view relay is answered here, on the connection's own thread: never on the main thread, so it
            // works while the app is busy and does not stall drawing.
            if method.hasPrefix("game.") {
                let reply = session.game.handle(method, params, fd: fd, client: clientName, stopped: { session.isStopped })
                if !send(fd, id: id, reply: reply) { return }
                continue
            }
            let box = ReplyBox()
            let done = DispatchSemaphore(value: 0)
            DispatchQueue.main.async {
                box.reply = LiveHandler.handle(method, params)
                done.signal()
            }
            let started = Date()
            var reply: LiveReply?
            while reply == nil {
                if done.wait(timeout: .now() + 0.25) == .success {
                    reply = box.reply
                } else if session.isStopped {
                    return
                } else if Date().timeIntervalSince(started) > Self.answerTimeout {
                    reply = .failure(.other("Pixmith did not answer in time"))
                }
            }
            if !send(fd, id: id, reply: reply!) { return }
        }
    }

    private final class ReplyBox {
        var reply: LiveReply?
    }

    private static func clip(_ value: Any?) -> String { String((value as? String ?? "").prefix(80)) }

    /// Checks a `hello` (nil params when the first message was something else). Returns the error, if any.
    private func hello(_ params: [String: Any]?, _ session: Session) -> LiveError? {
        guard let params else { return .unauthorized }
        if session.isBlocked { return LiveError(code: -32001, message: "disconnected by the user") }
        guard params["token"] as? String == session.token else { return LiveError(code: -32001, message: "wrong token") }
        if let p = params["protocol"] as? Int, p != Self.protocolVersion {
            return .params("protocol \(p) is not supported; this app speaks \(Self.protocolVersion)")
        }
        return nil
    }

    private static func helloResult() -> [String: Any] {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.4.0"
        return ["app": "Pixmith", "version": version, "front_end": "swift", "platform": "macos", "protocol": protocolVersion]
    }
}

// MARK: - Game view

/// Relays screenshots of a running game to the client that asked (`game.poll`, `game.frame`, `game.capture`). All of
/// its state sits behind one lock; `capture` waits on the calling connection's thread, never on the main thread.
final class GameRelay {
    static let pollWindow = 1.0
    static let defaultTimeout = 3000
    static let maxTimeout = 10000

    private final class Request {
        let id: String
        let fd: Int32
        let client: String
        var handedOut = false
        var answered = false
        var frame: [String: Any]?
        let done = DispatchSemaphore(value: 0)

        init(id: String, fd: Int32, client: String) {
            self.id = id
            self.fd = fd
            self.client = client
        }
    }

    private let lock = NSLock()
    private var lastFD: Int32?
    private var lastName = ""
    private var lastPoll = 0.0
    private var requests: [String: Request] = [:]
    private var serial = 0

    private static var now: Double { ProcessInfo.processInfo.systemUptime }

    func handle(_ method: String, _ params: [String: Any], fd: Int32, client: String, stopped: () -> Bool) -> LiveReply {
        switch method {
        case "game.poll":
            if let id = poll(fd: fd, client: client) { return .success(["capture": id]) }
            return .success(["capture": NSNull()])
        case "game.frame": return frame(params, fd: fd)
        case "game.capture": return capture(params, stopped: stopped)
        default: return .failure(LiveError(code: -32601, message: "unknown method \"\(method)\""))
        }
    }

    /// A connection ended: it is no longer a running game, and a capture waiting on it fails now.
    func forget(_ fd: Int32) {
        lock.lock()
        if lastFD == fd { lastFD = nil }
        let mine = requests.values.filter { $0.fd == fd }
        lock.unlock()
        for r in mine { r.done.signal() }
    }

    private func poll(fd: Int32, client: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        lastFD = fd
        lastName = client
        lastPoll = Self.now
        guard let r = requests.values.filter({ $0.fd == fd && !$0.handedOut }).min(by: { $0.id < $1.id }) else { return nil }
        r.handedOut = true
        return r.id
    }

    private func frame(_ params: [String: Any], fd: Int32) -> LiveReply {
        guard let id = params["request"] as? String else { return .failure(.params("request must be a string")) }
        lock.lock()
        // An unknown request, one already answered, or one never handed to this connection is dropped.
        guard let r = requests[id], r.fd == fd, r.handedOut, !r.answered else {
            lock.unlock()
            return .success([:])
        }
        guard let png = params["png"] as? String, !png.isEmpty else {
            lock.unlock()
            return .failure(.params("png must be a base64 string"))
        }
        guard let w = params["w"] as? Int, let h = params["h"] as? Int, w > 0, h > 0 else {
            lock.unlock()
            return .failure(.params("w and h must be positive integers"))
        }
        r.answered = true
        r.frame = ["png": png, "w": w, "h": h, "client": r.client]
        lock.unlock()
        r.done.signal()
        return .success([:])
    }

    private func capture(_ params: [String: Any], stopped: () -> Bool) -> LiveReply {
        var timeout = Self.defaultTimeout
        if let raw = params["timeout_ms"], !(raw is NSNull) {
            guard let t = raw as? Int, t > 0 else { return .failure(.params("timeout_ms must be a positive integer")) }
            timeout = min(t, Self.maxTimeout)
        }
        lock.lock()
        guard let fd = lastFD, Self.now - lastPoll <= Self.pollWindow else {
            lock.unlock()
            return .failure(.noGame("no running game: nothing polled in the last second"))
        }
        serial += 1
        let request = Request(id: "cap-\(serial)", fd: fd, client: lastName)
        requests[request.id] = request
        lock.unlock()

        let deadline = Date().addingTimeInterval(Double(timeout) / 1000)
        var reply: LiveReply?
        while reply == nil {
            let left = deadline.timeIntervalSinceNow
            if request.done.wait(timeout: .now() + min(max(left, 0), 0.25)) == .success {
                lock.lock()
                let frame = request.frame
                lock.unlock()
                if let frame {
                    reply = .success(frame)
                } else {
                    reply = .failure(.noGame("the game disconnected before it sent a frame"))
                }
            } else if stopped() {
                reply = .failure(.noGame("Live mode was turned off"))
            } else if left <= 0 {
                reply = .failure(.noGame("the game did not send a frame in time"))
            }
        }
        lock.lock()
        requests[request.id] = nil
        lock.unlock()
        return reply!
    }
}

// MARK: - Requests

/// Answers requests on the main thread. All document access of Live mode is here.
enum LiveHandler {
    private static var model: SpriteModel { SpriteModel.shared }
    private static var nextID = 0

    static func handle(_ method: String, _ params: [String: Any]) -> LiveReply {
        do {
            refresh()
            switch method {
            case "documents.list": return .success(list())
            case "documents.get": return .success(try get(params))
            case "godot.atlas": return .success(try godotAtlas(params))
            case "view.get": return .success(view(of: model.activeSlotObject ?? model.slots[0]))
            case "documents.apply", "documents.new", "documents.open", "documents.save", "documents.activate",
                 "documents.close", "documents.undo", "documents.redo", "view.set":
                break
            default: return .failure(LiveError(code: -32601, message: "unknown method \"\(method)\""))
            }
            // A document that does not exist is reported before the app being busy.
            let target: DocumentSlot?
            switch method {
            case "documents.new", "documents.open": target = nil
            case "view.set" where params["id"] == nil || params["id"] is NSNull: target = model.activeSlotObject
            default: target = try find(params)
            }
            if let why = busy() { return .failure(.busy("Pixmith is busy: \(why)")) }
            if let slot = target, slot === model.activeSlotObject {
                model.finishTransients()
                refresh()
            }
            let result: [String: Any]
            switch method {
            case "documents.apply": result = try apply(target!, params)
            case "documents.new": result = try newDocument(params)
            case "documents.open": result = try open(params)
            case "documents.save": result = try save(target!, params)
            case "documents.activate":
                AppRoute.shared.enterEditor()
                if let i = model.slots.firstIndex(where: { $0 === target }) { model.activate(i) }
                refresh()
                result = ["document": json(target!)]
            case "documents.close": result = try close(target!, params)
            case "documents.undo": result = try stepHistory(target!, params, redo: false)
            case "documents.redo": result = try stepHistory(target!, params, redo: true)
            default: result = try setView(target!, params)
            }
            refresh()
            return .success(result)
        } catch let e as LiveError {
            return .failure(e)
        } catch {
            return .failure(.other("\(error)"))
        }
    }

    // MARK: Documents and revisions

    private static func pointer(_ slot: DocumentSlot) -> OpaquePointer {
        slot === model.activeSlotObject ? model.doc : slot.doc
    }

    /// Bumps the revision of every tab that changed since the last look and gives new tabs their ids.
    static func refresh() {
        ChangeCoalescer.shared.flush()
        for slot in model.slots where !slot.closed {
            let active = slot === model.activeSlotObject
            let stamp = LiveStamp(revision: active ? model.revision : slot.revision, state: pixmith_state_id(pointer(slot)))
            // A tab whose document was replaced (the blank tab reused by New or Open, the last tab closed) is a new
            // document for clients: new id, revision 1.
            if slot.liveID == nil || slot.liveDocPtr != pointer(slot) {
                slot.liveDocPtr = pointer(slot)
                nextID += 1
                slot.liveID = "doc-\(nextID)"
                slot.liveRevision = 1
            } else if let seen = slot.liveSeen, seen != stamp {
                slot.liveRevision += 1
            }
            slot.liveSeen = stamp
        }
    }

    private static func find(_ params: [String: Any]) throws -> DocumentSlot {
        guard let id = params["id"] as? String else { throw LiveError.params("missing \"id\"") }
        guard let slot = model.slots.first(where: { $0.liveID == id && !$0.closed }) else {
            throw LiveError(code: -32004, message: "no open document \"\(id)\"")
        }
        return slot
    }

    private static func busy() -> String? {
        let m = model
        if MainWindow.hasSheet || NSApp.modalWindow != nil || m.showNewSheet || m.toolSheet != nil
            || m.renamingLayer != nil || m.errorMessage != nil || AppRoute.shared.sheet != nil
            || FilterState.shared.active != nil {
            return "a sheet is open"
        }
        return nil
    }

    private static func json(_ slot: DocumentSlot) -> [String: Any] {
        let doc = pointer(slot)
        let active = slot === model.activeSlotObject
        let path = active ? model.fileURL?.path : slot.fileURL?.path
        return [
            "id": slot.liveID ?? "",
            "name": model.name(of: slot),
            "path": path.map { $0 as Any } ?? NSNull(),
            "width": Int(pixmith_width(doc)),
            "height": Int(pixmith_height(doc)),
            "frames": Int(pixmith_frame_count(doc)),
            "layers": Int(pixmith_layer_count(doc)),
            "revision": slot.liveRevision,
            "dirty": pixmith_is_dirty(doc),
            "active": active,
            "layer": active ? model.layer : slot.layer,
            "frame": active ? model.frame : slot.frame,
        ]
    }

    private static func list() -> [String: Any] {
        let slots = model.slots.filter { !$0.closed }
        return ["documents": slots.map(json), "active": model.activeSlotObject?.liveID.map { $0 as Any } ?? NSNull()]
    }

    // MARK: Parameters

    private static func int(_ value: Any?) -> Int? {
        guard let n = value as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID() else { return nil }
        let d = n.doubleValue
        return d == d.rounded() && abs(d) < 9e15 ? Int(d) : nil
    }

    private static func requiredInt(_ params: [String: Any], _ key: String) throws -> Int {
        guard let v = params[key], !(v is NSNull) else { throw LiveError.params("missing \"\(key)\"") }
        guard let n = int(v) else { throw LiveError.params("\"\(key)\" must be an integer") }
        return n
    }

    private static func optionalInt(_ params: [String: Any], _ key: String) throws -> Int? {
        guard let v = params[key], !(v is NSNull) else { return nil }
        guard let n = int(v) else { throw LiveError.params("\"\(key)\" must be an integer") }
        return n
    }

    private static func string(_ params: [String: Any], _ key: String) throws -> String {
        guard let v = params[key], !(v is NSNull) else { throw LiveError.params("missing \"\(key)\"") }
        guard let s = v as? String else { throw LiveError.params("\"\(key)\" must be a string") }
        return s
    }

    private static func decodeBase64(_ text: String) -> Data? {
        var padded = text.filter { !$0.isWhitespace }
        while padded.count % 4 != 0 { padded.append("=") }
        return Data(base64Encoded: padded)
    }

    private static func absolutePath(_ params: [String: Any]) throws -> URL {
        let path = try string(params, "path")
        guard path.hasPrefix("/") else { throw LiveError.params("\"path\" must be an absolute path") }
        return URL(fileURLWithPath: path)
    }

    // MARK: Methods

    private static func get(_ params: [String: Any]) throws -> [String: Any] {
        let slot = try find(params)
        let doc = pointer(slot)
        var len = 0
        guard let bytes = pixmith_encode_document(doc, &len) else {
            throw LiveError.other(String(cString: pixmith_last_error()))
        }
        let encoded = Data(bytes: bytes, count: len).base64EncodedString()
        pixmith_bytes_free(bytes, len)
        var selection: Any = NSNull()
        var (x, y) = (Int32(0), Int32(0))
        var (w, h) = (UInt32(0), UInt32(0))
        var maskLen = 0
        if let mask = pixmith_live_selection(doc, &x, &y, &w, &h, &maskLen) {
            selection = ["x": Int(x), "y": Int(y), "w": Int(w), "h": Int(h),
                         "mask": Data(bytes: mask, count: maskLen).base64EncodedString()] as [String: Any]
            pixmith_bytes_free(mask, maskLen)
        }
        return ["document": json(slot), "pxmt": encoded, "selection": selection]
    }

    /// The Godot Live Link layout of a tab: the core's JSON (`pixmith_godot_link`) plus the document.
    private static func godotAtlas(_ params: [String: Any]) throws -> [String: Any] {
        let slot = try find(params)
        guard let p = pixmith_godot_link(pointer(slot)) else {
            throw LiveError.other(String(cString: pixmith_last_error()))
        }
        defer { pixmith_string_free(p) }
        guard var result = (try? JSONSerialization.jsonObject(with: Data(String(cString: p).utf8))) as? [String: Any] else {
            throw LiveError.other("the Godot layout is not valid JSON")
        }
        result["document"] = json(slot)
        return result
    }

    /// The optional `selection` of `documents.apply`: absent keeps it, null clears it, an object sets it.
    private enum SelectionChange {
        case keep, clear
        case set(x: Int, y: Int, w: Int, h: Int, mask: [UInt8])
    }

    private static func selectionChange(_ params: [String: Any]) throws -> SelectionChange {
        guard let value = params["selection"] else { return .keep }
        if value is NSNull { return .clear }
        func bad(_ what: String) -> LiveError { .params("\"selection\": \(what)") }
        guard let o = value as? [String: Any] else { throw bad("must be an object or null") }
        func field(_ k: String) throws -> Int {
            guard let n = int(o[k]) else { throw bad("missing or bad \"\(k)\"") }
            return n
        }
        let (x, y, w, h) = (try field("x"), try field("y"), try field("w"), try field("h"))
        guard w >= 0, h >= 0, w <= 1 << 16, h <= 1 << 16, abs(x) <= 1 << 20, abs(y) <= 1 << 20 else {
            throw bad("the rectangle is out of range")
        }
        guard let text = o["mask"] as? String, let data = decodeBase64(text) else { throw bad("\"mask\" is not base64") }
        guard data.count == w * h else { throw bad("\"mask\" has \(data.count) bytes, not w * h = \(w * h)") }
        return .set(x: x, y: y, w: w, h: h, mask: [UInt8](data))
    }

    private static func apply(_ slot: DocumentSlot, _ params: [String: Any]) throws -> [String: Any] {
        let base = try requiredInt(params, "base_revision")
        guard base == slot.liveRevision else {
            throw LiveError(code: -32009, message: "stale revision: the document is at revision \(slot.liveRevision), not \(base); fetch it again")
        }
        guard let bytes = decodeBase64(try string(params, "pxmt")) else { throw LiveError.params("\"pxmt\" is not base64") }
        let given = (params["label"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let label = "AI: " + (given.isEmpty ? "edit" : given)
        let (layer, frame) = (try optionalInt(params, "layer"), try optionalInt(params, "frame"))
        let change = try selectionChange(params)
        let doc = pointer(slot)
        let (mode, x, y, w, h, mask): (UInt32, Int32, Int32, UInt32, UInt32, [UInt8])
        switch change {
        case .keep: (mode, x, y, w, h, mask) = (0, 0, 0, 0, 0, [])
        case .clear: (mode, x, y, w, h, mask) = (1, 0, 0, 0, 0, [])
        case .set(let sx, let sy, let sw, let sh, let m): (mode, x, y, w, h, mask) = (2, Int32(sx), Int32(sy), UInt32(sw), UInt32(sh), m)
        }
        let ok = bytes.withUnsafeBytes { raw -> Bool in
            let p = raw.bindMemory(to: UInt8.self).baseAddress ?? UnsafePointer(bitPattern: 1)!
            return label.withCString { l in
                mask.withUnsafeBufferPointer {
                    pixmith_apply_document_selection(doc, p, bytes.count, l, mode, x, y, w, h, $0.baseAddress, mask.count)
                }
            }
        }
        guard ok else { throw LiveError.other(String(cString: pixmith_last_error())) }
        documentChanged(slot)
        if let layer { setLayer(slot, layer) }
        if let frame { setFrame(slot, frame) }
        refresh()
        return ["document": json(slot)]
    }

    /// What the app does after the core changed a document behind its back (the same as after an undo).
    private static func documentChanged(_ slot: DocumentSlot) {
        let m = model
        m.revision &+= 1
        HistoryStamp.value &+= 1
        if slot === m.activeSlotObject {
            m.opts.shape = nil
            m.composite = []
            m.clampSelection()
        } else {
            slot.revision = m.revision
            slot.layer = min(slot.layer, Int(pixmith_layer_count(slot.doc)) - 1)
            slot.frame = min(slot.frame, Int(pixmith_frame_count(slot.doc)) - 1)
        }
    }

    private static func setLayer(_ slot: DocumentSlot, _ layer: Int) {
        let n = Int(pixmith_layer_count(pointer(slot)))
        let value = min(max(layer, 0), max(n - 1, 0))
        if slot === model.activeSlotObject { model.layer = value } else { slot.layer = value }
    }

    private static func setFrame(_ slot: DocumentSlot, _ frame: Int) {
        let n = Int(pixmith_frame_count(pointer(slot)))
        let value = min(max(frame, 0), max(n - 1, 0))
        if slot === model.activeSlotObject {
            model.stop()
            model.frame = value
        } else {
            slot.frame = value
        }
    }

    private static func stepHistory(_ slot: DocumentSlot, _ params: [String: Any], redo: Bool) throws -> [String: Any] {
        let wanted = try optionalInt(params, "steps") ?? 1
        guard wanted >= 0 else { throw LiveError.params("\"steps\" must be 0 or more") }
        let doc = pointer(slot)
        var done = 0
        while done < min(wanted, 10_000), redo ? pixmith_redo(doc) : pixmith_undo(doc) { done += 1 }
        if done > 0 { documentChanged(slot) }
        refresh()
        return ["document": json(slot), "steps": done]
    }

    /// Shows a document a Live request made. A blank tab is reused only when a Live request did not make it (the
    /// app's own initial tab, once); the tab that results is marked so no later request reuses it.
    private static func presentFromLive(_ d: OpaquePointer, url: URL?) {
        if model.activeSlotObject?.liveCreated == true {
            model.newTab(doc: d, url: url)
        } else {
            model.present(d, url: url)
        }
        model.activeSlotObject?.liveCreated = true
    }

    private static func newDocument(_ params: [String: Any]) throws -> [String: Any] {
        // Everything is checked and built before a tab exists, so a request that fails (or whose client gives up)
        // never leaves a half-made document behind.
        func dimension(_ key: String) throws -> Int {
            let v = try requiredInt(params, key)
            guard (1...4096).contains(v) else { throw LiveError.params("\"\(key)\" must be 1 to 4096") }
            return v
        }
        var bytes: Data?
        if let v = params["pxmt"], !(v is NSNull) {
            guard let d = decodeBase64(try string(params, "pxmt")) else { throw LiveError.params("\"pxmt\" is not base64") }
            bytes = d
        }
        var name: String?
        if let v = params["name"], !(v is NSNull) {
            guard let s = v as? String else { throw LiveError.params("\"name\" must be a string") }
            let trimmed = s.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.contains(where: { "/\\:".contains($0) }) else { throw LiveError.params("\"name\" must not contain / \\ or :") }
            name = trimmed.isEmpty ? nil : trimmed
        }
        // A blank untitled tab the app opened itself is reused, else the document gets a new tab; either way it is
        // the active one.
        let built: OpaquePointer
        if let bytes {
            let d = bytes.withUnsafeBytes { raw -> OpaquePointer? in
                pixmith_decode_document(raw.bindMemory(to: UInt8.self).baseAddress ?? UnsafePointer(bitPattern: 1)!, bytes.count)
            }
            guard let d else { throw LiveError.other(String(cString: pixmith_last_error())) }
            built = d
        } else {
            built = pixmith_new(UInt32(try dimension("width")), UInt32(try dimension("height")))
        }
        presentFromLive(built, url: nil)
        model.activeSlotObject?.customName = name
        model.objectWillChange.send()
        refresh()
        return ["document": json(model.activeSlotObject!)]
    }

    private static func open(_ params: [String: Any]) throws -> [String: Any] {
        let url = try absolutePath(params)
        var isDir: ObjCBool = false
        if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue {
            throw LiveError.params("\"path\" is a folder; open a document file")
        }
        if url.pathExtension.lowercased() == "png" {
            guard let d = url.path.withCString({ pixmith_import_png($0) }) else { throw LiveError.other(model.lastError) }
            presentFromLive(d, url: nil)
            model.activeSlotObject?.importSource = url
            model.changed()
            RecentFiles.shared.add(url)
        } else if let slot = model.slot(for: url), let i = model.slots.firstIndex(where: { $0 === slot }) {
            AppRoute.shared.enterEditor()
            model.activate(i)
        } else {
            guard let d = url.path.withCString({ pixmith_open($0) }) else { throw LiveError.other(model.lastError) }
            presentFromLive(d, url: url)
        }
        refresh()
        return ["document": json(model.activeSlotObject!)]
    }

    private static func save(_ slot: DocumentSlot, _ params: [String: Any]) throws -> [String: Any] {
        var replaceOpen = false
        if let v = params["replace_open"], !(v is NSNull) {
            guard let n = v as? NSNumber, CFGetTypeID(n) == CFBooleanGetTypeID() else {
                throw LiveError.params("\"replace_open\" must be true or false")
            }
            replaceOpen = n.boolValue
        }
        var active = slot === model.activeSlotObject
        let url: URL
        if let v = params["path"], !(v is NSNull) {
            url = try absolutePath(params)
        } else if let own = active ? model.fileURL : slot.fileURL {
            url = own
        } else {
            throw LiveError.params("the document has no file yet; pass \"path\"")
        }
        guard ["pxmt", "aseprite", "ase"].contains(url.pathExtension.lowercased()) else {
            throw LiveError.params("save as .pxmt or .aseprite")
        }
        // Another tab holds the file: with `replace_open` a tab without unsaved changes is closed first.
        let key = url.resolvingSymlinksInPath()
        if replaceOpen, let other = model.slots.first(where: { $0 !== slot && $0.fileURL?.resolvingSymlinksInPath() == key }),
           !model.isDirty(other), let i = model.slots.firstIndex(where: { $0 === other }) {
            model.closeTab(i, discardChanges: true)
            active = slot === model.activeSlotObject
        }
        if active {
            if let message = model.writeChecked(to: url) { throw LiveError.other(message) }
        } else {
            if let other = model.slots.first(where: { $0 !== slot && $0.fileURL?.resolvingSymlinksInPath() == key }) {
                throw LiveError.other("“\(model.name(of: other))” is already open in another tab. Close that tab first.")
            }
            guard url.path.withCString({ pixmith_save(slot.doc, $0) }) else { throw LiveError.other(model.lastError) }
            slot.fileURL = url
            pixmith_mark_saved(slot.doc)
            slot.savedRevision = slot.revision
            Autosaver.shared.discard(slot)
            RecentFiles.shared.add(url)
        }
        refresh()
        return ["document": json(slot)]
    }

    private static func close(_ slot: DocumentSlot, _ params: [String: Any]) throws -> [String: Any] {
        var discard = false
        if let v = params["discard"], !(v is NSNull) {
            guard let n = v as? NSNumber, CFGetTypeID(n) == CFBooleanGetTypeID() else {
                throw LiveError.params("\"discard\" must be true or false")
            }
            discard = n.boolValue
        }
        if model.isDirty(slot), !discard {
            throw LiveError(code: -32011, message: "“\(model.name(of: slot))” has unsaved changes")
        }
        if let i = model.slots.firstIndex(where: { $0 === slot }) { model.closeTab(i, discardChanges: true) }
        return [:]
    }

    // MARK: View

    static func toolName(_ tool: Tool) -> String {
        switch tool {
        case .shapePolygon: "shape_polygon"
        default: tool.rawValue
        }
    }

    private static func view(of slot: DocumentSlot) -> [String: Any] {
        let active = slot === model.activeSlotObject
        let zoom = active ? model.zoom : slot.zoom
        return [
            "document": slot.liveID.map { $0 as Any } ?? NSNull(),
            "layer": active ? model.layer : slot.layer,
            "frame": active ? model.frame : slot.frame,
            "zoom": Double(zoom),
            "tool": toolName(model.tool),
        ]
    }

    private static func setView(_ slot: DocumentSlot, _ params: [String: Any]) throws -> [String: Any] {
        let (layer, frame) = (try optionalInt(params, "layer"), try optionalInt(params, "frame"))
        if let layer { setLayer(slot, layer) }
        if let frame { setFrame(slot, frame) }
        return view(of: slot)
    }
}

// MARK: - The "AI connected" chip

/// A quiet chip in the title row while an AI client is connected. Its menu names the client and offers Disconnect.
struct LiveChip: View {
    @ObservedObject private var live = LiveServer.shared
    @State private var hovering = false

    var body: some View {
        if !live.clients.isEmpty {
            Menu {
                ForEach(live.clients) { c in
                    Text(c.name.isEmpty ? "A client" : c.name + (c.version.isEmpty ? "" : " \(c.version)"))
                }
                Text("Its edits are undo steps.")
                Divider()
                Button("Disconnect") { live.disconnectByUser() }
            } label: {
                HStack(spacing: DS.Space.xs + 1) {
                    Circle().fill(DSColor.accent).frame(width: 6, height: 6)
                    Text("AI connected").font(DS.Font.captionMedium)
                }
                .foregroundStyle(DSColor.accent)
                .padding(.horizontal, DS.Space.sm)
                .frame(height: DS.Metric.control)
                .background(RoundedRectangle(cornerRadius: DS.Radius.control).fill(hovering ? DSColor.selectedHover : DSColor.selected))
                .contentShape(Rectangle())
                .onHover { hovering = $0 }
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("An AI agent is connected. Click for details.")
            .accessibilityLabel("AI connected")
        }
    }
}
