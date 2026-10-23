-- Add a tag for every `size` frames ("Part 1", "Part 2", ...); every second one ping-pongs.
local size = 4
local count = 0

for first = 1, sprite.frameCount, size do
  local last = math.min(first + size - 1, sprite.frameCount)
  count = count + 1
  local tag = sprite:newTag("Part " .. count, first, last)
  if count % 2 == 0 then
    tag.direction = app.direction.PingPong
  end
end

app.undoLabel("Tag every " .. size .. " frames")
app.alert("Added " .. count .. " tags")
