-- A plugin: loaded once when the extension is enabled. `init` registers commands, which appear under
-- File > Scripts. The Dialog API is described in docs/LUA_DIALOG.md, the sprite API in docs/LUA_API.md.

function init(plugin)
  plugin.preferences.runs = plugin.preferences.runs or 0

  plugin:newCommand{
    id = "sample_dots",
    title = "Sample: Diagonal Dots...",
    onenabled = function() return sprite ~= nil end,
    onclick = function()
      local dlg = Dialog("Diagonal dots")
      dlg:color  { id = "color", label = "Color", color = 0xFF6AC1FF }
         :slider { id = "count", label = "Dots", min = 1, max = 16, value = 8 }
         :button { id = "ok", text = "&OK", focus = true }
         :button { id = "cancel", text = "Cancel" }
         :show()
      local d = dlg.data
      if not d.ok then return end
      local n = math.min(d.count, sprite.width, sprite.height)
      for i = 0, n - 1 do
        sprite:setPixel(i, i, d.color)
      end
      plugin.preferences.runs = plugin.preferences.runs + 1
      app.alert("Drew " .. n .. " dots (run " .. plugin.preferences.runs .. ")")
    end,
  }
end

function exit(plugin)
  -- Optional: runs when the extension is disabled, uninstalled or the app quits.
end
