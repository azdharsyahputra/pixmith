-- Generate a new layer holding a left-to-right color ramp from `from` to `to`.
-- Edit the two colors below; each is 0xRRGGBBAA.
local from = 0x1D2B53FF
local to = 0xFFA300FF

local function lerp(a, b, t)
  return math.floor(a + (b - a) * t + 0.5)
end

local r1, g1, b1, a1 = color.split(from)
local r2, g2, b2, a2 = color.split(to)

local layer = sprite:newLayer("Color ramp")
local last = layer.index
local w = sprite.width
for x = 0, w - 1 do
  local t = w > 1 and x / (w - 1) or 0
  local c = color.rgba(lerp(r1, r2, t), lerp(g1, g2, t), lerp(b1, b2, t), lerp(a1, a2, t))
  for y = 0, sprite.height - 1 do
    sprite:setPixel(x, y, c, last, sprite.activeFrame)
  end
end

app.activeLayer = layer.index   -- select the new layer in the editor when the script ends
app.undoLabel("Color ramp")
