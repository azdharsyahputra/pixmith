-- A plugin: it is loaded once (not run), defines init(plugin) / exit(plugin) and adds a command to the
-- File > Scripts menu (see docs/LUA_DIALOG.md). Preferences saved in plugin.preferences survive restarts.

function init(plugin)
  plugin.preferences.uses = plugin.preferences.uses or 0

  plugin:newCommand{
    id = "mark-corner",
    title = "Mark Top-Left Corner...",
    group = "file_scripts",
    onenabled = function() return sprite ~= nil end,
    onclick = function()
      local dlg = Dialog("Mark corner")
      dlg:color { id = "c", label = "Color", color = plugin.preferences.color or 0xFF004DFF }
         :newrow()
         :label { text = ("Used %d times before."):format(plugin.preferences.uses) }
         :newrow()
         :button{ id = "ok", text = "Mark", focus = true }
         :button{ id = "cancel", text = "Cancel" }
         :show{ wait = true }
      local d = dlg.data
      if d.ok then
        plugin.preferences.uses = plugin.preferences.uses + 1
        plugin.preferences.color = d.c
        sprite:setPixel(0, 0, d.c)
        app.undoLabel("Mark corner")
      end
    end,
  }

  -- Events: report the active layer and frame whenever the site changes.
  app.events:on("sitechange", function(ev)
    print("site: layer " .. app.activeLayer.stackIndex .. ", frame " .. app.activeFrame.frameNumber)
  end)
end

function exit(plugin)
  -- Nothing to clean up; preferences are saved automatically.
end
