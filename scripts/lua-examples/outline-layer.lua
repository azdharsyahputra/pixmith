-- Outline the current layer: every transparent pixel that touches an opaque one
-- (up, down, left or right) is painted with the outline color.
local outline = color.rgba(0, 0, 0, 255)
local layer, frame = sprite.activeLayer, sprite.activeFrame
local w, h = sprite.width, sprite.height

local function opaque(x, y)
  local _, _, _, a = color.split(sprite:getPixel(x, y, layer, frame))
  return a > 0
end

-- Collect first, then paint, so the outline never grows into itself.
local todo = {}
for y = 0, h - 1 do
  for x = 0, w - 1 do
    if not opaque(x, y) and (opaque(x - 1, y) or opaque(x + 1, y) or opaque(x, y - 1) or opaque(x, y + 1)) then
      todo[#todo + 1] = { x, y }
    end
  end
end

for _, p in ipairs(todo) do
  sprite:setPixel(p[1], p[2], outline, layer, frame)
end

app.undoLabel("Outline layer")
app.alert("Outlined " .. #todo .. " pixels")
