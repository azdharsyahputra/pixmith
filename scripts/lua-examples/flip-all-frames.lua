-- Flip every frame of every drawable layer horizontally.
local w, h = sprite.width, sprite.height
local flipped = 0

for li = 1, sprite.layerCount do
  if not sprite.layers[li].isGroup then
    for fi = 1, sprite.frameCount do
      -- Linked cels are one shared image: flip it once, from its first frame.
      local cel = sprite:cel(li, fi)
      local first = true
      for _, other in ipairs(cel.linkedFrames) do
        if other < fi then first = false end
      end
      if first and not cel.isEmpty then
        local px = sprite:getPixels(li, fi)
        local out = {}
        for y = 0, h - 1 do
          for x = 0, w - 1 do
            out[y * w + x + 1] = px[y * w + (w - 1 - x) + 1]
          end
        end
        sprite:setPixels(li, fi, out)
        flipped = flipped + 1
      end
    end
  end
end

app.undoLabel("Flip all frames")
app.alert("Flipped " .. flipped .. " cels")
