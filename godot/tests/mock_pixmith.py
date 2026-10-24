#!/usr/bin/env python3
"""A stand-in for the Pixmith app's Live server (docs/LIVE_PROTOCOL.md), for testing the Godot addon without Pixmith.

Python 3.8+, standard library only.

    mock_pixmith.py --support-dir DIR [--mode linked|unlinked] [--chunk N] [--big] [--decoys]

It writes DIR/Pixmith/Live/<pid>.json (DIR/pixmith/... on Linux, like the app; point PIXMITH_SUPPORT_DIR at DIR), listens
on 127.0.0.1, and answers hello, documents.list and godot.atlas for two documents: "hero.pxmt" (the one the
demo game uses) and "other.pxmt" (which the game does not have).

hero.pxmt shows a numbered *variant* of its sprite, solid 8x8 colour blocks whose colour and layout depend on the
variant. DIR/mock_state.json says what the mock currently shows, so a test can compare pixels.

Control, by writing one line into DIR/mock.cmd (the mock reads and deletes it):
    bump     the next variant: revision + 1 and a different atlas
    restart  the app "quits" and starts again: drops every connection, removes the discovery file, waits a moment,
             then re-executes this program with a new port and token, revision 1 and the next variant
    capture  ask the game for a screenshot: game.poll then returns a request id ("cap-N") until the game answers
             with game.frame; the mock checks the PNG and logs "frame request=cap-N w=.. h=.. png=ok"
    quit     exits

Everything the mock sees is appended to DIR/mock.log (hello with its client name, list, atlas, bad tokens).
"""
import argparse
import base64
import json
import os
import random
import secrets
import signal
import socket
import socketserver
import struct
import sys
import threading
import time
import zlib

BLOCK = 8
TIP = "Godot defaults to Linear texture filtering, which blurs pixel art."


def png(w, h, rgba):
    raw = b"".join(b"\x00" + bytes(rgba[y * w * 4:(y + 1) * w * 4]) for y in range(h))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def color(variant, i):
    return ((70 + 95 * variant + 30 * i) % 256, (200 - 45 * variant + 40 * i) % 256, (90 + 60 * variant + 70 * i) % 256)


def layout(variant, big):
    """(page width, page height, rgba, animations, summary) for a variant. The shape repeats every three variants:
    0: idle with 2 frames; 1: idle with 3 frames plus run; 2: idle with 2 trimmed frames (a margin)."""
    shape = variant % 3
    n = 3 if shape == 1 else 2
    w, h = BLOCK * n, BLOCK + (256 if big else 0)
    rgba = bytearray(w * h * 4)
    if big:  # incompressible noise below the frames, to make the message large
        rng = random.Random(variant)
        noise = bytes(rng.getrandbits(8) for _ in range(w * 256 * 4))
        rgba[w * BLOCK * 4:] = noise
    size = 6 if shape == 2 else BLOCK
    regions = []
    for i in range(n):
        r, g, b = color(variant, i)
        for y in range(size):
            for x in range(size):
                o = (y * w + i * BLOCK + x) * 4
                rgba[o:o + 4] = bytes((r, g, b, 255))
        regions.append([i * BLOCK, 0, size, size])
    margin = [1, 1, BLOCK - size, BLOCK - size] if shape == 2 else None

    def frame(i, duration):
        return {"page": 0, "region": regions[i], "margin": margin if i == 0 else None, "duration": duration}

    if shape == 0:
        anims = [{"name": "idle", "loop": True, "speed": 5.0, "frames": [frame(0, 1.0), frame(1, 1.0)]}]
    elif shape == 1:
        anims = [{"name": "idle", "loop": True, "speed": 8.0, "frames": [frame(0, 1.0), frame(1, 1.0), frame(2, 2.0)]},
                 {"name": "run", "loop": True, "speed": 12.0, "frames": [frame(2, 1.0), frame(0, 1.0)]}]
    else:
        anims = [{"name": "idle", "loop": True, "speed": 10.0, "frames": [frame(0, 1.0), frame(1, 1.0)]}]
    summary = {"frame0": list(color(variant, 0)), "animations": {a["name"]: len(a["frames"]) for a in anims},
               "speed": anims[0]["speed"], "margin0": margin, "region0": regions[0]}
    return w, h, rgba, anims, summary


class Mock:
    def __init__(self, args):
        self.args = args
        self.lock = threading.Lock()
        self.dir = os.path.abspath(args.support_dir)
        folder = "pixmith" if sys.platform.startswith("linux") else "Pixmith"
        self.live = os.path.join(self.dir, folder, "Live")
        self.state_path = os.path.join(self.dir, "mock_state.json")
        self.cmd_path = os.path.join(self.dir, "mock.cmd")
        self.log_path = os.path.join(self.dir, "mock.log")
        self.discovery = os.path.join(self.live, "%d.json" % os.getpid())
        self.token = secrets.token_hex(16)
        self.boot = secrets.token_hex(4)
        self.revision = 1
        self.variant = 0
        if os.path.exists(self.state_path):
            try:
                self.variant = int(json.load(open(self.state_path)).get("next_variant", 0))
            except (ValueError, OSError):
                pass
        self.other_revision = 1
        self.capture = None
        self.captures = 0
        self.polls = 0
        self.conns = []
        self.server = None
        self.files = []

    def log(self, text):
        with open(self.log_path, "a") as f:
            f.write(text + "\n")

    def write_state(self):
        s = layout(self.variant, False)[4]
        s.update({"variant": self.variant, "revision": self.revision, "boot": self.boot, "pid": os.getpid(),
                  "next_variant": self.variant})
        tmp = self.state_path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(s, f)
        os.replace(tmp, self.state_path)

    def doc(self, ident, name, revision):
        return {"id": ident, "name": name, "path": "/tmp/" + name, "width": 8, "height": 8, "frames": 2,
                "layers": 1, "revision": revision, "dirty": True, "active": ident == "doc-1", "layer": 0, "frame": 0}

    def documents(self):
        with self.lock:
            return [self.doc("doc-1", "hero.pxmt", self.revision), self.doc("doc-2", "other.pxmt", self.other_revision)]

    def atlas(self, ident):
        with self.lock:
            variant, revision = self.variant, self.revision
        if ident == "doc-2":
            w, h, rgba, anims, _ = layout(1, False)
            return {"document": self.doc("doc-2", "other.pxmt", self.other_revision), "linked": self.args.mode == "linked",
                    "scale": 1, "warnings": [], "resources": [{
                        "resource": "res://sprites/other.tres" if self.args.mode == "linked" else None,
                        "pages": [{"w": w, "h": h, "png": base64.b64encode(png(w, h, rgba)).decode()}],
                        "animations": anims}]}
        if ident != "doc-1":
            return None
        w, h, rgba, anims, _ = layout(variant, self.args.big)
        linked = self.args.mode == "linked"
        return {"document": self.doc("doc-1", "hero.pxmt", revision), "linked": linked, "scale": 1, "warnings": [TIP],
                "resources": [{"resource": "res://sprites/hero.tres" if linked else None,
                               "pages": [{"w": w, "h": h, "png": base64.b64encode(png(w, h, rgba)).decode()}],
                               "animations": anims}]}

    def send(self, sock, obj):
        data = (json.dumps(obj, separators=(",", ":")) + "\n").encode()
        if self.args.chunk > 0:
            for i in range(0, len(data), self.args.chunk):
                sock.sendall(data[i:i + self.args.chunk])
                time.sleep(0.003)
        else:
            sock.sendall(data)

    def handle_client(self, sock):
        authed = False
        client = "?"
        with self.lock:
            self.conns.append(sock)
        try:
            for raw in sock.makefile("rb"):
                try:
                    req = json.loads(raw.decode("utf-8"))
                    rid, method, params = req.get("id"), req.get("method"), req.get("params") or {}
                except (ValueError, AttributeError):
                    self.send(sock, {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}})
                    continue
                if not authed:
                    if method != "hello" or params.get("token") != self.token:
                        self.log("hello refused client=%s" % params.get("client"))
                        self.send(sock, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32001, "message": "unauthorized"}})
                        return
                    client = params.get("client")
                    authed = True
                    self.log("hello ok client=%s protocol=%s" % (client, params.get("protocol")))
                    self.send(sock, {"jsonrpc": "2.0", "id": rid, "result": {
                        "app": "Pixmith", "version": "0.0.0-mock", "front_end": "mock", "platform": "test", "protocol": 1}})
                    continue
                if method == "documents.list":
                    self.log("list")
                    self.send(sock, {"jsonrpc": "2.0", "id": rid,
                                     "result": {"documents": self.documents(), "active": "doc-1"}})
                elif method == "godot.atlas":
                    result = self.atlas(params.get("id"))
                    if result is None:
                        self.send(sock, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32004, "message": "unknown document"}})
                    else:
                        self.log("atlas %s rev=%d variant=%d" % (params.get("id"), result["document"]["revision"], self.variant))
                        self.send(sock, {"jsonrpc": "2.0", "id": rid, "result": result})
                elif method == "game.poll":
                    with self.lock:
                        self.polls += 1
                        first, cap = self.polls == 1, self.capture
                    if first:
                        self.log("gamepoll client=%s" % client)
                    self.send(sock, {"jsonrpc": "2.0", "id": rid, "result": {"capture": cap}})
                elif method == "game.frame":
                    self.on_frame(sock, rid, params)
                else:
                    self.send(sock, {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "unknown method"}})
        except OSError:
            pass
        finally:
            with self.lock:
                if sock in self.conns:
                    self.conns.remove(sock)
            try:
                sock.close()
            except OSError:
                pass

    def on_frame(self, sock, rid, p):
        with self.lock:
            known = p.get("request") == self.capture and self.capture is not None
            if known:
                self.capture = None
        if not known:  # unknown or already answered: dropped, answered with {}
            self.log("frame dropped request=%s" % p.get("request"))
            self.send(sock, {"jsonrpc": "2.0", "id": rid, "result": {}})
            return
        verdict = "ok"
        try:
            raw = base64.b64decode(p["png"], validate=True)
            if raw[:8] != b"\x89PNG\r\n\x1a\n":
                verdict = "bad-signature"
            else:
                w, h = struct.unpack(">II", raw[16:24])
                if (w, h) != (p.get("w"), p.get("h")):
                    verdict = "size-mismatch %dx%d" % (w, h)
                else:
                    # decode the pixels: every IDAT inflates to h rows of 1 + w * channels bytes
                    pos, idat, channels = 8, b"", None
                    while pos < len(raw):
                        n = struct.unpack(">I", raw[pos:pos + 4])[0]
                        kind = raw[pos + 4:pos + 8]
                        if kind == b"IHDR":
                            channels = {2: 3, 6: 4, 0: 1}[raw[pos + 8 + 9]]
                        elif kind == b"IDAT":
                            idat += raw[pos + 8:pos + 8 + n]
                        pos += 12 + n
                    if len(zlib.decompress(idat)) != h * (1 + w * channels):
                        verdict = "bad-pixel-data"
        except Exception as e:  # noqa
            verdict = "invalid (%s)" % e
        self.log("frame request=%s w=%s h=%s png=%s" % (p.get("request"), p.get("w"), p.get("h"), verdict))
        self.send(sock, {"jsonrpc": "2.0", "id": rid, "result": {}})

    def start(self):
        os.makedirs(self.live, exist_ok=True)
        mock = self

        class H(socketserver.BaseRequestHandler):
            def handle(self):
                self.request.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                mock.handle_client(self.request)

        class S(socketserver.ThreadingTCPServer):
            allow_reuse_address = True
            daemon_threads = True

        self.server = S(("127.0.0.1", 0), H)
        port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.write_discovery(port, self.token, os.getpid(), int(time.time()), self.discovery)
        if self.args.decoys:
            # Newest first: a dead port, then the live port with a wrong token, then garbage. A client has to get
            # past all three to the real file.
            closed = socket.socket()
            closed.bind(("127.0.0.1", 0))
            dead = closed.getsockname()[1]
            closed.close()
            now = int(time.time())
            self.write_discovery(dead, "d" * 32, 99999, now + 300, os.path.join(self.live, "99999.json"))
            self.write_discovery(port, "e" * 32, 99998, now + 200, os.path.join(self.live, "99998.json"))
            with open(os.path.join(self.live, "99997.json"), "w") as f:
                f.write("{not json")
        self.write_state()
        self.log("started port=%d variant=%d" % (port, self.variant))

    def write_discovery(self, port, token, pid, started, path):
        doc = {"protocol": 1, "pid": pid, "port": port, "token": token, "app": "Pixmith", "version": "0.0.0-mock",
               "front_end": "mock", "platform": "test", "started": started}
        with open(path, "w") as f:
            json.dump(doc, f)
        os.chmod(path, 0o600)

    def stop(self):
        for name in os.listdir(self.live) if os.path.isdir(self.live) else []:
            if name in ("%d.json" % os.getpid(), "99999.json", "99998.json", "99997.json"):
                try:
                    os.remove(os.path.join(self.live, name))
                except OSError:
                    pass
        if self.server:
            self.server.shutdown()
            self.server.server_close()
        with self.lock:
            for c in list(self.conns):
                try:
                    c.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass

    def commands(self):
        while True:
            time.sleep(0.05)
            try:
                with open(self.cmd_path) as f:
                    text = f.read()
            except OSError:
                continue
            if not text.endswith("\n"):
                continue
            os.remove(self.cmd_path)
            cmd = text.strip()
            self.log("cmd " + cmd)
            if cmd == "bump":
                with self.lock:
                    self.variant += 1
                    self.revision += 1
                self.write_state()
            elif cmd == "capture":
                with self.lock:
                    self.captures += 1
                    self.capture = "cap-%d" % self.captures
            elif cmd == "restart":
                self.stop()
                # next process shows the next variant
                s = json.load(open(self.state_path))
                s["next_variant"] = self.variant + 1
                with open(self.state_path + ".tmp", "w") as f:
                    json.dump(s, f)
                os.replace(self.state_path + ".tmp", self.state_path)
                time.sleep(self.args.restart_delay)
                os.execv(sys.executable, [sys.executable] + sys.argv)
            elif cmd == "quit":
                self.stop()
                os._exit(0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--support-dir", required=True)
    ap.add_argument("--mode", choices=["linked", "unlinked"], default="linked")
    ap.add_argument("--chunk", type=int, default=0, help="send answers in pieces of N bytes with pauses")
    ap.add_argument("--big", action="store_true", help="add incompressible noise to the atlas page (a large message)")
    ap.add_argument("--decoys", action="store_true", help="also write newer discovery files that must be skipped")
    ap.add_argument("--restart-delay", type=float, default=1.0)
    args = ap.parse_args()
    mock = Mock(args)
    mock.start()

    def bye(*_):
        mock.stop()
        os._exit(0)

    signal.signal(signal.SIGTERM, bye)
    signal.signal(signal.SIGINT, bye)
    mock.commands()


if __name__ == "__main__":
    main()
