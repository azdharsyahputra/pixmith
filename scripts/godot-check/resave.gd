extends SceneTree
# Lets Godot save resources/scenes itself so its text can be compared with ours.
# Usage: godot --headless --path <proj> --script res://resave.gd -- <in> <out> [<in> <out> ...]
# Each saved file also gets a fresh uid, like the editor does.

func _init() -> void:
	var args := OS.get_cmdline_user_args()
	var failed := false
	for i in range(0, args.size(), 2):
		var res = load(args[i])
		if res == null:
			printerr("FAIL: cannot load ", args[i])
			failed = true
			continue
		DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(args[i + 1].get_base_dir()))
		var err := ResourceSaver.save(res, args[i + 1])
		if err == OK:
			err = ResourceSaver.set_uid(args[i + 1], ResourceUID.create_id())
		if err != OK:
			printerr("FAIL: saving ", args[i + 1], " error ", err)
			failed = true
	quit(1 if failed else 0)
