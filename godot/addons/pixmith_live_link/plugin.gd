@tool
extends EditorPlugin
## Registers the Live Link autoload while the plugin is enabled. The autoload itself (live_link.gd) only runs in
## debug builds of the game, never in the editor and never in an exported release.

const AUTOLOAD_NAME := "PixmithLiveLink"
const AUTOLOAD_PATH := "res://addons/pixmith_live_link/live_link.gd"
const SETTING_ENABLED := "pixmith/live_link/enabled"


func _enter_tree() -> void:
	# Show the on/off switch in Project Settings. Unchanged it is not written to project.godot.
	if not ProjectSettings.has_setting(SETTING_ENABLED):
		ProjectSettings.set_setting(SETTING_ENABLED, true)
	ProjectSettings.set_initial_value(SETTING_ENABLED, true)
	ProjectSettings.add_property_info({"name": SETTING_ENABLED, "type": TYPE_BOOL})


func _enable_plugin() -> void:
	add_autoload_singleton(AUTOLOAD_NAME, AUTOLOAD_PATH)


func _disable_plugin() -> void:
	remove_autoload_singleton(AUTOLOAD_NAME)
