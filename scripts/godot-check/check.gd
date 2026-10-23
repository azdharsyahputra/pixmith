extends SceneTree
# Loads SpriteFrames resources / scenes exported by pixmith-cli and checks them against manifest.json.
# Usage: godot --headless --path <proj> --script res://check.gd -- res://art/a.tres res://art/b.tscn ...
# The first .tres argument is the reference; others must render the same full-canvas frames.

var failures := 0
var manifest: Dictionary
var canvas: Vector2
var reference: Dictionary = {}  # "anim:index" -> PackedByteArray


func fail(msg: String) -> void:
	failures += 1
	printerr("FAIL: " + msg)


func _init() -> void:
	manifest = JSON.parse_string(FileAccess.get_file_as_string("res://manifest.json"))
	canvas = Vector2(manifest.canvas[0], manifest.canvas[1])
	var first := true
	for path in OS.get_cmdline_user_args():
		if path.ends_with(".tscn"):
			check_scene(path)
		else:
			check_frames(path, first)
			first = false
	if failures == 0:
		print("OK: all checks passed")
	quit(1 if failures > 0 else 0)


func frame_canvas(tex: AtlasTexture) -> PackedByteArray:
	var atlas_img := tex.atlas.get_image()
	if atlas_img == null or atlas_img.is_empty():
		return PackedByteArray()
	atlas_img.convert(Image.FORMAT_RGBA8)
	var out := Image.create(int(canvas.x), int(canvas.y), false, Image.FORMAT_RGBA8)
	out.blit_rect(atlas_img, Rect2i(tex.region), Vector2i(tex.margin.position))
	# Godot's importer rewrites the color of fully transparent pixels (fix_alpha_border); ignore it.
	var data := out.get_data()
	for i in range(0, data.size(), 4):
		if data[i + 3] == 0:
			data[i] = 0
			data[i + 1] = 0
			data[i + 2] = 0
	return data


func check_frames(path: String, is_reference: bool) -> void:
	var sf = load(path)
	if sf == null or not (sf is SpriteFrames):
		fail("%s did not load as SpriteFrames" % path)
		return
	var anims: Array = manifest.animations
	var names = sf.get_animation_names()
	var expected_count: int = anims.size()
	if "default" in names:
		expected_count += 1  # SpriteFrames always owns a "default" animation when created in the editor
	if names.size() != anims.size() and names.size() != expected_count:
		fail("%s: expected %d animations, found %d (%s)" % [path, anims.size(), names.size(), names])
	for a in anims:
		var n := StringName(a.name)
		if not sf.has_animation(n):
			fail("%s: missing animation %s" % [path, a.name])
			continue
		var ms: Array = a.ms
		if sf.get_frame_count(n) != ms.size():
			fail("%s/%s: expected %d frames, found %d" % [path, a.name, ms.size(), sf.get_frame_count(n)])
			continue
		if sf.get_animation_loop(n) != a.loop:
			fail("%s/%s: loop should be %s" % [path, a.name, a.loop])
		var speed: float = sf.get_animation_speed(n)
		for i in ms.size():
			var real_ms: float = sf.get_frame_duration(n, i) / speed * 1000.0
			if absf(real_ms - float(ms[i])) > 0.01:
				fail("%s/%s[%d]: %.4f ms, expected %d" % [path, a.name, i, real_ms, ms[i]])
			var tex = sf.get_frame_texture(n, i)
			if not (tex is AtlasTexture) or tex.atlas == null:
				fail("%s/%s[%d]: not an AtlasTexture with an atlas" % [path, a.name, i])
				continue
			if tex.get_size() != canvas:
				fail("%s/%s[%d]: size %s should be %s (margin restores the canvas)" % [path, a.name, i, tex.get_size(), canvas])
			var key := "%s:%d" % [a.name, i]
			var pixels := frame_canvas(tex)
			if pixels.is_empty():
				continue  # no readable image in this renderer
			if is_reference:
				reference[key] = pixels
			elif reference.has(key) and reference[key] != pixels:
				fail("%s/%s[%d]: pixels differ from the reference export" % [path, a.name, i])
	print("checked ", path)


func check_scene(path: String) -> void:
	var packed = load(path)
	if packed == null or not (packed is PackedScene):
		fail("%s did not load as PackedScene" % path)
		return
	var node = packed.instantiate()
	if not (node is AnimatedSprite2D):
		fail("%s: root is not an AnimatedSprite2D" % path)
		return
	if node.texture_filter != CanvasItem.TEXTURE_FILTER_NEAREST:
		fail("%s: texture_filter is not nearest" % path)
	if node.sprite_frames == null:
		fail("%s: sprite_frames not set" % path)
	elif not node.sprite_frames.has_animation(node.animation):
		fail("%s: animation %s does not exist" % [path, node.animation])
	node.free()
	print("checked ", path)
