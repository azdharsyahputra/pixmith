extends Node
## Pixmith Live Link (autoload).
##
## Talks to the Pixmith pixel-art editor running on the same computer (docs/LIVE_PROTOCOL.md in the Pixmith repo) and
## swaps the SpriteFrames of the running game for what is being drawn there: no save, export or restart.
##
## It does nothing in a release build, in the editor itself, or when switched off (Project Settings
## "pixmith/live_link/enabled", or the environment variable PIXMITH_LIVE_LINK=0). The network code never blocks:
## the socket is polled once per frame.

## Emitted after the hello handshake with Pixmith succeeded.
signal connected(app_version: String)
## Emitted when an established connection is lost (Pixmith quit, or Live mode was switched off).
signal disconnected
## Emitted after a SpriteFrames was updated: its resource path, the Pixmith document name and revision.
signal resource_applied(target: String, document: String, revision: int)

enum State { OFF, IDLE, CONNECTING, HELLO, READY }

const SETTING_ENABLED := "pixmith/live_link/enabled"
const CLIENT_NAME := "pixmith-godot"
const CLIENT_VERSION := "1.0.0"
const PROTOCOL := 1

const POLL_MS := 150            ## documents.list interval
const RETRY_MS := 2000          ## wait before looking for Pixmith again
const CONNECT_TIMEOUT_MS := 3000
const REQUEST_TIMEOUT_MS := 30000
const REAPPLY_MS := 500         ## how often stored sprites are matched against what the game has loaded
const MAX_FRAME_SIDE := 1280      ## game.frame screenshots are scaled down to this long side
const MAX_BUFFER_BYTES := 80 * 1024 * 1024  ## the protocol allows 64 MiB per message

const META_STAMP := &"pixmith_live_stamp"

var state: State = State.OFF
var app_version := ""
## Number of SpriteFrames updates applied since the game started.
var applied_count := 0

var _peer: StreamPeerTCP
var _buf := PackedByteArray()
var _scan := 0
var _candidates: Array = []
var _cand_i := 0
var _round_active := false
var _current: Dictionary = {}
var _next_try_ms := 0
var _connect_started_ms := 0
var _next_id := 0
var _inflight: Dictionary = {}
var _queue: Array = []
var _next_poll_ms := 0
var _next_reapply_ms := 0
var _fetched: Dictionary = {}   ## document id -> revision we last fetched
var _payloads: Dictionary = {}  ## document id -> {doc, resources, serial}
var _serial := 0
var _blocked_tokens: Dictionary = {}
var _warned: Dictionary = {}
var _want_game_poll := false
var _game_supported := true
var _capturing := false
var _last_capture := ""
var _enc_task := -1
var _enc_result: Dictionary = {}
var _pending_frame: Dictionary = {}


## True when the link may run at all: a debug build, not the editor itself, and switched on.
static func is_allowed(debug_build: bool, editor_hint: bool, enabled: bool) -> bool:
	return debug_build and not editor_hint and enabled


## Size of a game.frame screenshot: at most MAX_FRAME_SIDE on the long side, never enlarged.
static func frame_size(w: int, h: int) -> Vector2i:
	var longest := maxi(w, h)
	if longest <= MAX_FRAME_SIDE or longest <= 0:
		return Vector2i(maxi(w, 1), maxi(h, 1))
	var k := float(MAX_FRAME_SIDE) / float(longest)
	return Vector2i(maxi(roundi(w * k), 1), maxi(roundi(h * k), 1))


## The two off switches: the project setting and the PIXMITH_LIVE_LINK environment variable.
static func config_enabled() -> bool:
	var env := OS.get_environment("PIXMITH_LIVE_LINK").strip_edges().to_lower()
	if env in ["0", "false", "off", "no"]:
		return false
	return bool(ProjectSettings.get_setting(SETTING_ENABLED, true))


## Pixmith's app data folder (where the Live/<pid>.json discovery files are), see LIVE_PROTOCOL.md.
static func support_dir() -> String:
	var os_name := OS.get_name()
	var mac := os_name == "macOS" or os_name == "OSX"
	var win := os_name == "Windows"
	var folder := "Pixmith" if (mac or win) else "pixmith"
	var override := OS.get_environment("PIXMITH_SUPPORT_DIR")
	if override != "":
		return override.path_join(folder)
	if mac:
		return OS.get_environment("HOME").path_join("Library/Application Support").path_join(folder)
	if win:
		return OS.get_environment("APPDATA").path_join(folder)
	var xdg := OS.get_environment("XDG_DATA_HOME")
	if xdg != "":
		return xdg.path_join(folder)
	return OS.get_environment("HOME").path_join(".local/share").path_join(folder)


## The usable discovery files, newest `started` first.
static func read_discovery(dir: String) -> Array:
	var live := dir.path_join("Live")
	var out: Array = []
	var d := DirAccess.open(live)
	if d == null:
		return out
	for f in d.get_files():
		if not f.ends_with(".json"):
			continue
		var json := JSON.new()
		if json.parse(FileAccess.get_file_as_string(live.path_join(f))) != OK:
			continue
		var data = json.data
		if not (data is Dictionary):
			continue
		var port := int(data.get("port", 0))
		if port <= 0 or port > 65535 or str(data.get("token", "")) == "":
			continue
		if int(data.get("protocol", PROTOCOL)) != PROTOCOL:
			continue
		out.append({"port": port, "token": str(data["token"]), "pid": int(data.get("pid", 0)),
				"started": int(data.get("started", 0)), "file": f})
	out.sort_custom(func(a, b): return a.started > b.started or (a.started == b.started and a.pid > b.pid))
	return out


func _ready() -> void:
	process_mode = Node.PROCESS_MODE_ALWAYS
	if OS.has_feature("web") or not is_allowed(OS.is_debug_build(), Engine.is_editor_hint(), config_enabled()):
		set_process(false)
		return
	state = State.IDLE


func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE and _peer != null:
		_peer.disconnect_from_host()


## One line for a debug overlay.
func status_text() -> String:
	match state:
		State.OFF:
			return "off"
		State.READY:
			return "connected to Pixmith %s, %d update(s)" % [app_version, applied_count]
		_:
			return "waiting for Pixmith"


# --- connection ---------------------------------------------------------------------------------------------------

func _process(_delta: float) -> void:
	var now := Time.get_ticks_msec()
	match state:
		State.IDLE:
			_step_idle(now)
		State.CONNECTING:
			_step_connecting(now)
		State.HELLO, State.READY:
			_step_connected(now)
	if _enc_task != -1 and WorkerThreadPool.is_task_completed(_enc_task):
		WorkerThreadPool.wait_for_task_completion(_enc_task)
		_enc_task = -1
		_capturing = false
		if state == State.READY:
			_pending_frame = _enc_result
		_enc_result = {}
	if now >= _next_reapply_ms:
		_next_reapply_ms = now + REAPPLY_MS
		_reapply_all()


func _step_idle(now: int) -> void:
	if now < _next_try_ms:
		return
	if not _round_active:
		_candidates = read_discovery(support_dir()).filter(func(c): return not _blocked_tokens.has(c.token))
		_cand_i = 0
		_round_active = true
	if _cand_i >= _candidates.size():
		_round_active = false
		_next_try_ms = now + RETRY_MS
		return
	_current = _candidates[_cand_i]
	_cand_i += 1
	_peer = StreamPeerTCP.new()
	if _peer.connect_to_host("127.0.0.1", _current.port) != OK:
		_peer = null
		return
	state = State.CONNECTING
	_connect_started_ms = now


func _step_connecting(now: int) -> void:
	_peer.poll()
	match _peer.get_status():
		StreamPeerTCP.STATUS_CONNECTED:
			state = State.HELLO
			_connect_started_ms = now
			_send("hello", {"token": _current.token, "client": CLIENT_NAME, "client_version": CLIENT_VERSION,
					"protocol": PROTOCOL}, {})
		StreamPeerTCP.STATUS_CONNECTING:
			if now - _connect_started_ms > CONNECT_TIMEOUT_MS:
				_close(true)
		_:
			_close(true)


func _step_connected(now: int) -> void:
	_peer.poll()
	if _peer.get_status() != StreamPeerTCP.STATUS_CONNECTED:
		_close(state != State.READY)
		return
	_read()
	if _peer == null:
		return
	if not _inflight.is_empty():
		var limit := CONNECT_TIMEOUT_MS if _inflight.method == "hello" else REQUEST_TIMEOUT_MS
		if now - int(_inflight.sent_ms) > limit:
			_close(state != State.READY)
		return
	if state != State.READY:
		return
	if not _pending_frame.is_empty():
		var f := _pending_frame
		_pending_frame = {}
		_send("game.frame", f, {})
	elif _want_game_poll:
		_want_game_poll = false
		_send("game.poll", {}, {})
	elif not _queue.is_empty():
		var job: Dictionary = _queue.pop_front()
		_send("godot.atlas", {"id": job.id}, job)
	elif now >= _next_poll_ms:
		_send("documents.list", {}, {})


## Drops the connection. `try_next` continues with the next discovery file at once (a candidate that failed);
## otherwise this was an established session and a quiet retry follows after RETRY_MS.
func _close(try_next: bool) -> void:
	var was_ready := state == State.READY
	if _peer != null:
		_peer.disconnect_from_host()
	_peer = null
	_buf = PackedByteArray()
	_scan = 0
	_inflight = {}
	_queue.clear()
	_pending_frame = {}
	_want_game_poll = false
	state = State.IDLE
	if not try_next:
		_round_active = false
		_next_try_ms = Time.get_ticks_msec() + RETRY_MS
	if was_ready:
		disconnected.emit()


func _send(method: String, params: Dictionary, ctx: Dictionary) -> void:
	_next_id += 1
	var line := JSON.stringify({"jsonrpc": "2.0", "id": _next_id, "method": method, "params": params}) + "\n"
	_inflight = {"id": _next_id, "method": method, "ctx": ctx, "sent_ms": Time.get_ticks_msec()}
	if _peer.put_data(line.to_utf8_buffer()) != OK:
		_close(state != State.READY)


## Reads what has arrived and handles every complete line. Partial lines wait in _buf for their newline.
func _read() -> void:
	var avail := _peer.get_available_bytes()
	if avail < 0:
		_close(state != State.READY)
		return
	if avail > 0:
		var r: Array = _peer.get_data(avail)
		if r[0] != OK:
			_close(state != State.READY)
			return
		_buf.append_array(r[1])
		if _buf.size() > MAX_BUFFER_BYTES:
			_warn_once("too_big", "Pixmith Live Link: a message from Pixmith is too large, connection dropped")
			_close(state != State.READY)
			return
	while true:
		var nl := _buf.find(10, _scan)
		if nl < 0:
			_scan = _buf.size()
			return
		var line := _buf.slice(0, nl)
		_buf = _buf.slice(nl + 1)
		_scan = 0
		_on_line(line.get_string_from_utf8())
		if _peer == null:
			return


func _on_line(text: String) -> void:
	if text.strip_edges() == "":
		return
	var json := JSON.new()
	if json.parse(text) != OK or not (json.data is Dictionary):
		_warn_once("bad_json", "Pixmith Live Link: unreadable message from Pixmith, connection dropped")
		_close(state != State.READY)
		return
	var msg: Dictionary = json.data
	if _inflight.is_empty() or int(msg.get("id", -1)) != int(_inflight.id):
		return
	var req := _inflight
	_inflight = {}
	if msg.get("error") is Dictionary:
		_on_error(req, msg["error"])
	else:
		_on_result(req, msg.get("result"))


func _on_error(req: Dictionary, err: Dictionary) -> void:
	var code := int(err.get("code", 0))
	var message := str(err.get("message", ""))
	match req.method:
		"hello":
			# "disconnected by the user": do not come back with this token (a new launch or a Live mode toggle
			# in Pixmith writes a new one).
			if code == -32001 and message.contains("disconnected by the user"):
				_blocked_tokens[_current.token] = true
			elif code != -32001:
				_warn_once("hello_%d" % code, "Pixmith Live Link: Pixmith refused the connection (%d: %s)" % [code, message])
			_close(true)
		"godot.atlas":
			# Remember the revision so a failing document is not asked again until it changes.
			_fetched[req.ctx.id] = int(req.ctx.rev)
			if code != -32004:
				_warn_once("atlas_%d" % code, "Pixmith Live Link: godot.atlas failed (%d: %s)" % [code, message])
		"game.poll":
			_game_supported = false  # an older Pixmith without the game view
		"game.frame":
			pass  # the request was unknown or already answered
		_:
			_warn_once("list_%d" % code, "Pixmith Live Link: documents.list failed (%d: %s)" % [code, message])
			_next_poll_ms = Time.get_ticks_msec() + RETRY_MS


func _on_result(req: Dictionary, result) -> void:
	if not (result is Dictionary):
		_warn_once("bad_result", "Pixmith Live Link: unexpected answer from Pixmith")
		_close(state != State.READY)
		return
	match req.method:
		"hello":
			if int(result.get("protocol", PROTOCOL)) != PROTOCOL:
				_warn_once("protocol", "Pixmith Live Link: Pixmith speaks another protocol version")
				_close(true)
				return
			app_version = str(result.get("version", ""))
			state = State.READY
			_round_active = false
			# A new connection may be a different Pixmith session: fetch everything again.
			_fetched.clear()
			_queue.clear()
			_next_poll_ms = 0
			_game_supported = true
			_want_game_poll = false
			print("[Pixmith Live Link] connected to Pixmith %s" % app_version)
			connected.emit(app_version)
		"documents.list":
			_on_documents(result)
			_next_poll_ms = Time.get_ticks_msec() + POLL_MS
			_want_game_poll = _game_supported
		"game.poll":
			var request = result.get("capture")
			if request is String and request != "" and request != _last_capture and not _capturing:
				_last_capture = request
				_begin_capture(request)
		"game.frame":
			pass
		"godot.atlas":
			_on_atlas(req.ctx, result)


func _on_documents(result: Dictionary) -> void:
	var docs = result.get("documents", [])
	if not (docs is Array):
		return
	var open_ids := {}
	for d in docs:
		if not (d is Dictionary):
			continue
		var id := str(d.get("id", ""))
		var rev := int(d.get("revision", 0))
		open_ids[id] = true
		if _fetched.get(id, -1) == rev:
			continue
		var queued := false
		for job in _queue:
			if job.id == id:
				job.rev = rev
				queued = true
		if not queued:
			_queue.append({"id": id, "rev": rev})
	# A closed tab stops being applied; what the game shows stays as it is.
	for id in _payloads.keys():
		if not open_ids.has(id):
			_payloads.erase(id)
			_fetched.erase(id)


func _on_atlas(ctx: Dictionary, result: Dictionary) -> void:
	var doc = result.get("document", {})
	var resources = result.get("resources", [])
	if not (doc is Dictionary) or not (resources is Array):
		return
	var id: String = ctx.id
	_fetched[id] = int(doc.get("revision", ctx.rev))
	_serial += 1
	_payloads[id] = {"doc": doc, "resources": resources, "serial": _serial}
	_apply_payload(id)


# --- game view ----------------------------------------------------------------------------------------------------

## Pixmith asked for a screenshot of the game: grab the viewport once the frame is drawn, then scale and encode it
## on a worker thread; _process sends the game.frame answer.
func _begin_capture(request: String) -> void:
	_capturing = true
	var headless := DisplayServer.get_name() == "headless"
	if headless:
		await get_tree().process_frame
	else:
		await RenderingServer.frame_post_draw
	var vp := get_viewport()
	var img: Image = null
	if not headless:  # the dummy renderer has no pixels to read
		img = vp.get_texture().get_image()
	if img == null or img.is_empty():
		# No pixels (headless): answer with a black image of the viewport's size so the request is still served.
		img = Image.create(maxi(vp.size.x, 1), maxi(vp.size.y, 1), false, Image.FORMAT_RGB8)
	_enc_result = {}
	_enc_task = WorkerThreadPool.add_task(_encode_frame.bind(request, img))


func _encode_frame(request: String, img: Image) -> void:
	var size := frame_size(img.get_width(), img.get_height())
	if size != img.get_size():
		img.resize(size.x, size.y, Image.INTERPOLATE_BILINEAR)
	_enc_result = {"request": request, "png": Marshalls.raw_to_base64(img.save_png_to_buffer()),
			"w": img.get_width(), "h": img.get_height()}


# --- applying -----------------------------------------------------------------------------------------------------

## Tries again, without the network, for every stored sprite: the game may have loaded the resource since, or
## loaded a fresh copy of it from disk.
func _reapply_all() -> void:
	for id in _payloads.keys():
		_apply_payload(id)


func _apply_payload(id: String) -> void:
	var p: Dictionary = _payloads[id]
	var resources: Array = p.resources
	var doc: Dictionary = p.doc
	var doc_stem := str(doc.get("name", "")).get_basename()
	for k in resources.size():
		var r = resources[k]
		if not (r is Dictionary):
			continue
		for sf in _targets(r, k, doc_stem):
			# Newer payloads win; a SpriteFrames already at (or past) this one is left alone.
			if int(sf.get_meta(META_STAMP, 0)) >= int(p.serial):
				continue
			if _patch(sf, r):
				sf.set_meta(META_STAMP, int(p.serial))
				applied_count += 1
				resource_applied.emit(sf.resource_path, str(doc.get("name", "")), int(doc.get("revision", 0)))


## The SpriteFrames the game uses for a resource of the atlas answer. With a res:// path: the cached resource of
## that path. Without one (the document was never exported to Godot): every SpriteFrames of an AnimatedSprite
## whose file name, without .tres, equals the document name without its extension.
func _targets(r: Dictionary, index: int, doc_stem: String) -> Array:
	var out: Array = []
	var path = r.get("resource")
	if path is String and path != "":
		if ResourceLoader.has_cached(path):
			var res = load(path)
			if res is SpriteFrames:
				out.append(res)
		return out
	if index != 0 or doc_stem == "":
		return out
	for n in _sprite_nodes():
		var sf = n.sprite_frames
		if sf is SpriteFrames and sf.resource_path != "" and not sf.resource_path.contains("::") \
				and sf.resource_path.get_file().get_basename() == doc_stem and not out.has(sf):
			out.append(sf)
	return out


func _sprite_nodes() -> Array:
	var root := get_tree().root
	var nodes: Array = root.find_children("*", "AnimatedSprite2D", true, false)
	nodes.append_array(root.find_children("*", "AnimatedSprite3D", true, false))
	return nodes


## Rewrites `sf` in place so it equals the atlas answer `r`: pages become ImageTextures, every frame's AtlasTexture gets the new atlas, region and margin, and frames and animations are
## added or removed to match. Nodes that play it keep playing.
func _patch(sf: SpriteFrames, r: Dictionary) -> bool:
	var pages_in = r.get("pages", [])
	var anims_in = r.get("animations", [])
	if not (pages_in is Array) or not (anims_in is Array):
		return false
	# 1. decode every page first, so a bad PNG changes nothing
	var images: Array = []
	for p in pages_in:
		var img := Image.new()
		if not (p is Dictionary) or img.load_png_from_buffer(Marshalls.base64_to_raw(str(p.get("png", "")))) != OK:
			_warn_once("png", "Pixmith Live Link: could not decode an atlas page from Pixmith")
			return false
		if img.get_format() != Image.FORMAT_RGBA8:
			img.convert(Image.FORMAT_RGBA8)
		images.append(img)
	# 2. page textures: always new ones (pixel-art atlases are small), so every AtlasTexture sees a change
	var pages: Array = []
	for img in images:
		pages.append(ImageTexture.create_from_image(img))
	# 3. animations and frames
	var wanted := {}
	var used := {}  # AtlasTextures already placed, so one object is never given two different regions
	for a in anims_in:
		if not (a is Dictionary) or str(a.get("name", "")) == "":
			continue
		var anim := StringName(str(a["name"]))
		wanted[anim] = true
		if not sf.has_animation(anim):
			sf.add_animation(anim)
		sf.set_animation_speed(anim, float(a.get("speed", 5.0)))
		sf.set_animation_loop(anim, bool(a.get("loop", true)))
		var frames = a.get("frames", [])
		if not (frames is Array):
			frames = []
		var before := sf.get_frame_count(anim)
		var slot := 0
		for f in frames:
			if not (f is Dictionary):
				continue
			var page := int(f.get("page", 0))
			var region = f.get("region")
			if page < 0 or page >= pages.size() or not (region is Array) or region.size() < 4:
				_warn_once("frame", "Pixmith Live Link: a frame from Pixmith points outside its atlas and was skipped")
				continue
			var tex: AtlasTexture = null
			if slot < before:
				var old = sf.get_frame_texture(anim, slot)
				if old is AtlasTexture and not used.has(old):
					tex = old
			var fresh := tex == null
			if fresh:
				tex = AtlasTexture.new()
			used[tex] = true
			tex.atlas = pages[page]
			tex.region = _rect(region)
			var margin = f.get("margin")
			tex.margin = _rect(margin) if (margin is Array and margin.size() >= 4) else Rect2()
			var duration := float(f.get("duration", 1.0))
			if slot < before:
				if fresh or not is_equal_approx(sf.get_frame_duration(anim, slot), duration):
					sf.set_frame(anim, slot, tex, duration)
			else:
				sf.add_frame(anim, tex, duration)
			slot += 1
		for j in range(before - 1, slot - 1, -1):
			sf.remove_frame(anim, j)
	for n in sf.get_animation_names():
		if not wanted.has(StringName(n)):
			sf.remove_animation(n)
	sf.emit_changed()
	_fix_nodes(sf)
	return true


static func _rect(a: Array) -> Rect2:
	return Rect2(float(a[0]), float(a[1]), float(a[2]), float(a[3]))


## Nodes showing `sf` whose animation was removed switch to the first one (and keep playing); a frame index past
## the end moves to the last frame.
func _fix_nodes(sf: SpriteFrames) -> void:
	for n in _sprite_nodes():
		if n.sprite_frames != sf:
			continue
		var anim: StringName = n.animation
		if not sf.has_animation(anim):
			var names := sf.get_animation_names()
			if names.is_empty():
				continue
			var was_playing: bool = n.is_playing()
			n.animation = StringName(names[0])
			if was_playing:
				n.play()
		else:
			var count := sf.get_frame_count(anim)
			if count > 0 and n.frame >= count:
				n.frame = count - 1


func _warn_once(kind: String, message: String) -> void:
	if _warned.has(kind):
		return
	_warned[kind] = true
	push_warning(message)
