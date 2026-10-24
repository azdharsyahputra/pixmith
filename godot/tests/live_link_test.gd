extends SceneTree
## Headless test of the Pixmith Live Link addon against tests/mock_pixmith.py. Started by run.sh:
##   godot --headless --path godot/demo -s godot/tests/live_link_test.gd -- --support-dir=DIR --scenario=flow|disabled
##        [--mode=linked|unlinked] [--decoys]
## The game side is the real demo scene; the mock is steered by writing DIR/mock.cmd and read through
## DIR/mock_state.json and DIR/mock.log.

var support := ""
var scenario := "flow"
var mode := "linked"
var decoys := false
var failures: Array[String] = []
var link: Node
var hero: AnimatedSprite2D
var disconnects := 0
const OFF := 0     # the link's State enum
const READY := 4


func _initialize() -> void:
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--support-dir="):
			support = a.trim_prefix("--support-dir=")
		elif a.begins_with("--scenario="):
			scenario = a.trim_prefix("--scenario=")
		elif a.begins_with("--mode="):
			mode = a.trim_prefix("--mode=")
		elif a == "--decoys":
			decoys = true
	_run()


func _run() -> void:
	link = root.get_node_or_null("PixmithLiveLink")
	check(link != null, "the PixmithLiveLink autoload is registered")
	if link == null:
		return _finish()
	var main: Node = load("res://main.tscn").instantiate()
	root.add_child(main)
	hero = main.get_node("Hero")
	check(main.get_script() != null, "the demo scene script loads")
	link.disconnected.connect(func(): disconnects += 1)
	if scenario == "disabled":
		await _disabled()
	else:
		await _flow()
	_finish()


func _finish() -> void:
	for f in failures:
		printerr("FAIL: " + f)
	print("%s (%s): %s" % [scenario, mode, "FAILED" if failures.size() > 0 else "ok"])
	quit(1 if failures.size() > 0 else 0)


func check(cond: bool, label: String) -> void:
	if cond:
		print("  pass: " + label)
	else:
		failures.append(label)
		printerr("  FAIL: " + label)


func wait_until(cond: Callable, seconds: float) -> bool:
	var end := Time.get_ticks_msec() + int(seconds * 1000.0)
	while Time.get_ticks_msec() < end:
		if cond.call():
			return true
		await process_frame
	return cond.call()


func read_state() -> Dictionary:
	var f := FileAccess.open(support.path_join("mock_state.json"), FileAccess.READ)
	if f == null:
		return {}
	var json := JSON.new()
	if json.parse(f.get_as_text()) != OK or not (json.data is Dictionary):
		return {}
	return json.data


func read_log() -> String:
	return FileAccess.get_file_as_string(support.path_join("mock.log"))


func command(cmd: String) -> void:
	var f := FileAccess.open(support.path_join("mock.cmd"), FileAccess.WRITE)
	f.store_string(cmd + "\n")
	f.close()


func hex(rgb: Array) -> String:
	return Color8(int(rgb[0]), int(rgb[1]), int(rgb[2])).to_html(false)


## Colour in the middle of a frame's texture, as hex; "" when the image cannot be read.
func center_hex(anim: String, idx: int) -> String:
	var sf := hero.sprite_frames
	if sf == null or not sf.has_animation(anim) or idx >= sf.get_frame_count(anim):
		return ""
	var tex := sf.get_frame_texture(anim, idx)
	var img: Image = tex.get_image() if tex != null else null
	if img == null or img.is_empty():
		return ""
	return img.get_pixel(img.get_width() / 2, img.get_height() / 2).to_html(false)


func names() -> Array:
	var out := []
	for n in hero.sprite_frames.get_animation_names():
		out.append(String(n))
	out.sort()
	return out


func expect_state(st: Dictionary, label: String) -> void:
	var want: Dictionary = st.animations
	var keys := want.keys()
	keys.sort()
	check(names() == keys, "%s: animations are %s" % [label, keys])
	for a in want:
		check(hero.sprite_frames.get_frame_count(a) == int(want[a]), "%s: '%s' has %d frames" % [label, a, int(want[a])])
	check(is_equal_approx(hero.sprite_frames.get_animation_speed("idle"), float(st.speed)), "%s: idle speed %s" % [label, st.speed])
	var t0 := hero.sprite_frames.get_frame_texture("idle", 0)
	check(t0 is AtlasTexture, "%s: idle frame 0 is an AtlasTexture" % label)
	if t0 is AtlasTexture:
		var rg: Array = st.region0
		check(t0.region == Rect2(rg[0], rg[1], rg[2], rg[3]), "%s: idle frame 0 region %s" % [label, t0.region])
		var m = st.margin0
		check(t0.margin == (Rect2(m[0], m[1], m[2], m[3]) if m != null else Rect2()), "%s: idle frame 0 margin %s" % [label, t0.margin])


func playing_idle(label: String) -> void:
	check(hero.is_playing() and hero.animation == &"idle", "%s: the sprite is still playing 'idle'" % label)


func _flow() -> void:
	var st := read_state()
	check(not st.is_empty(), "the mock is up")
	if st.is_empty():
		return
	check(names() == ["idle", "walk"], "before the link: the game has the exported idle and walk")
	var disk_color := center_hex("idle", 0)

	# 1. connect and take over the sprites
	check(await wait_until(func(): return link.state == READY, 10.0), "the link connects to the mock")
	var ok: bool = await wait_until(func(): return center_hex("idle", 0) == hex(st.frame0), 10.0)
	check(ok, "first sync: idle frame 0 pixels are the mock's colour %s (found %s)" % [hex(st.frame0), center_hex("idle", 0)])
	if disk_color != "":
		check(disk_color != hex(st.frame0), "first sync: the pixels differ from what was loaded from disk")
	expect_state(st, "first sync")
	playing_idle("first sync")
	var log := read_log()
	check(log.contains("hello ok client=pixmith-godot protocol=1"), "hello names the client pixmith-godot")
	check(log.contains("atlas doc-1"), "the link asked godot.atlas for hero.pxmt")
	if decoys:
		check(log.contains("hello refused"), "the newer decoy files were tried and skipped")

	# 2. the user draws: revision + 1 and a different atlas
	var before := center_hex("idle", 0)
	command("bump")
	check(await wait_until(func(): return int(read_state().get("variant", -1)) == int(st.variant) + 1, 5.0), "the mock bumped")
	var st2 := read_state()
	ok = await wait_until(func(): return center_hex("idle", 0) == hex(st2.frame0), 10.0)
	check(ok, "after a bump: idle frame 0 pixels changed to %s (found %s)" % [hex(st2.frame0), center_hex("idle", 0)])
	if before != "":
		check(before != center_hex("idle", 0), "after a bump: the pixels differ from before")
	expect_state(st2, "after a bump")
	playing_idle("after a bump")

	# 2b. game view: Pixmith asks for a screenshot of the game and gets a PNG frame with the request id
	check(link.frame_size(3000, 1500) == Vector2i(1280, 640), "screenshots are scaled to 1280 px on the long side")
	check(link.frame_size(720, 1600) == Vector2i(576, 1280), "... also when the height is the long side")
	check(link.frame_size(640, 360) == Vector2i(640, 360), "... and small ones are not enlarged")
	check(read_log().contains("gamepoll client=pixmith-godot"), "the link calls game.poll next to documents.list")
	command("capture")
	ok = await wait_until(func(): return read_log().contains("frame request=cap-1 "), 10.0)
	check(ok, "the link answers the capture request with a game.frame")
	var want: Vector2i = link.frame_size(root.size.x, root.size.y)
	check(read_log().contains("frame request=cap-1 w=%d h=%d png=ok" % [want.x, want.y]),
			"the frame is a valid PNG of the viewport's size %s (headless: a black image, the dummy renderer has no pixels)" % want)
	await create_timer(0.6).timeout
	check(read_log().count("frame request=cap-1 ") == 1 and not read_log().contains("frame dropped"),
			"the request is answered exactly once")
	check(link.state == READY, "the link is still connected after a capture")

	# 3. Pixmith quits and starts again: new port, new token, revision 1 again
	var boot: String = st2.boot
	command("restart")
	check(await wait_until(func(): return disconnects >= 1, 8.0), "the link notices the lost connection")
	check(hero.sprite_frames.get_frame_count("idle") == int(st2.animations.idle), "while disconnected the last sprites stay")
	check(await wait_until(func(): return read_state().get("boot", boot) != boot, 12.0), "the mock restarted")
	var st3 := read_state()
	check(int(st3.get("revision", 0)) == 1, "the restarted mock is at revision 1")
	ok = await wait_until(func(): return center_hex("idle", 0) == hex(st3.frame0), 15.0)
	check(ok, "after the restart: pixels changed to %s (found %s)" % [hex(st3.frame0), center_hex("idle", 0)])
	check(link.state == READY, "the link is connected again")
	expect_state(st3, "after the restart")
	playing_idle("after the restart")

	# 4. and again, now at the very revision the link already fetched (1): only the new session tells it apart
	boot = st3.boot
	command("restart")
	check(await wait_until(func(): return read_state().get("boot", boot) != boot, 12.0), "the mock restarted again")
	var st4 := read_state()
	check(int(st4.get("revision", 0)) == int(st3.revision), "the second restart is at the same revision")
	ok = await wait_until(func(): return center_hex("idle", 0) == hex(st4.frame0), 15.0)
	check(ok, "after the second restart: pixels changed to %s (found %s)" % [hex(st4.frame0), center_hex("idle", 0)])
	expect_state(st4, "after the second restart")
	playing_idle("after the second restart")
	var hellos := read_log().count("hello ok client=pixmith-godot")
	check(hellos >= 3, "hello was sent on every new connection (%d)" % hellos)
	check(link.applied_count >= 4, "applied %d updates" % link.applied_count)


func _disabled() -> void:
	check(link.state == OFF, "switched off: the link is idle")
	check(not link.is_allowed(false, false, true), "a release build never runs the link")
	check(not link.is_allowed(true, true, true), "the editor itself never runs the link")
	check(not link.is_allowed(true, false, false), "the setting turns the link off")
	check(link.is_allowed(true, false, true), "a debug build of the game runs the link")
	var st := read_state()
	var disk_color := center_hex("idle", 0)
	await create_timer(2.5).timeout
	check(not read_log().contains("hello"), "switched off: Pixmith never saw a connection")
	check(disk_color == center_hex("idle", 0), "switched off: the sprite is untouched")
	check(names() == ["idle", "walk"], "switched off: the game's own animations remain")
	check(st.has("variant"), "the mock was up all along")
