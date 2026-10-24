extends Node2D
## Demo scene: a hero playing res://sprites/hero.tres. Space or click switches the animation.
## With Pixmith open and the hero drawn there, the sprite changes as you draw (see the addon's README).

@onready var hero: AnimatedSprite2D = $Hero
@onready var status: Label = $Status


func _process(_delta: float) -> void:
	var link := get_node_or_null("/root/PixmithLiveLink")
	var text := "Animation: %s (space switches)\n" % hero.animation
	if link == null:
		text += "Live Link: addon not installed"
	else:
		text += "Live Link: " + link.status_text()
	status.text = text


func _unhandled_input(event: InputEvent) -> void:
	var pressed := false
	if event is InputEventKey:
		pressed = event.pressed and not event.echo and event.keycode == KEY_SPACE
	elif event is InputEventMouseButton:
		pressed = event.pressed
	if not pressed:
		return
	var names := hero.sprite_frames.get_animation_names()
	if names.is_empty():
		return
	var i := names.find(String(hero.animation))
	hero.play(StringName(names[(i + 1) % names.size()]))
