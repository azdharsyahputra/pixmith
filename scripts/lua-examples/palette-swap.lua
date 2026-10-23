-- Replace colors in every cel of the sprite. Edit the table: [old color] = new color,
-- colors are 0xRRGGBBAA. Keep the new colors out of the left-hand side, so running the
-- script twice (or on linked cels, which share one image) changes nothing more.
local swaps = {
  [0xFF004DFF] = 0xFF77A8FF, -- red -> pink
  [0x29ADFFFF] = 0x00E436FF, -- blue -> green
}

local changed = 0
for li = 1, sprite.layerCount do
  if not sprite.layers[li].isGroup then
    for fi = 1, sprite.frameCount do
      local px = sprite:getPixels(li, fi)
      local dirty = false
      for i = 1, #px do
        local new = swaps[px[i]]
        if new then
          px[i] = new
          dirty = true
          changed = changed + 1
        end
      end
      if dirty then
        sprite:setPixels(li, fi, px)
      end
    end
  end
end

app.undoLabel("Palette swap")
app.alert("Replaced " .. changed .. " pixels")
