-- Add a color ramp to the palette. Asks for the two end colors and the number of steps in a dialog
-- (see docs/LUA_DIALOG.md); a live preview of the ramp is shown as a row of swatches.
local from, to, steps = 0x1D2B53FF, 0xFFA300FF, 8

local function lerp(a, b, t) return math.floor(a + (b - a) * t + 0.5) end

local function ramp(c1, c2, n)
  local r1, g1, b1, a1 = color.split(c1)
  local r2, g2, b2, a2 = color.split(c2)
  local out = {}
  for i = 0, n - 1 do
    local t = n > 1 and i / (n - 1) or 0
    out[#out + 1] = color.rgba(lerp(r1, r2, t), lerp(g1, g2, t), lerp(b1, b2, t), lerp(a1, a2, t))
  end
  return out
end

local dlg = Dialog("Palette ramp")
local function refresh()
  local d = dlg.data
  dlg:modify{ id = "preview", colors = ramp(d.from, d.to, d.steps) }
end

dlg:color { id = "from", label = "From", color = from, onchange = refresh }
   :color { id = "to", label = "To", color = to, onchange = refresh }
   :slider{ id = "steps", label = "Steps", min = 2, max = 32, value = steps, onchange = refresh }
   :shades{ id = "preview", colors = ramp(from, to, steps) }
   :separator()
   :button{ id = "ok", text = "Add to palette", focus = true }
   :button{ id = "cancel", text = "Cancel" }
   :show{ wait = true }

local d = dlg.data
if d.ok then
  local added = 0
  for _, c in ipairs(ramp(d.from, d.to, d.steps)) do
    sprite:addPaletteColor(c)
    added = added + 1
  end
  app.undoLabel("Palette ramp")
  app.alert(("Added %d colors to the palette."):format(added))
end
