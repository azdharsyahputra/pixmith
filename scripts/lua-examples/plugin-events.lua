-- A plugin that listens to editor events (docs/LUA_API.md, "Events" and "Plugins").
-- It counts site changes, color changes and edits of the active document, remembers the totals in
-- plugin.preferences, and adds File > Scripts > "Event Report..." and a checkable "Count Edits" command.

local counts = { sites = 0, colors = 0, edits = 0, undos = 0 }
local watching -- the listener id of the sprite.events registration for the current document

function init(plugin)
  local prefs = plugin.preferences
  prefs.sessions = (prefs.sessions or 0) + 1
  prefs.totalEdits = prefs.totalEdits or 0
  if prefs.countEdits == nil then prefs.countEdits = true end

  -- Listen to the sprite of the current site: when the site changes, move the sprite listener along.
  app.events:on("sitechange", function(ev)
    counts.sites = counts.sites + 1
    if watching then
      -- The old registration belongs to the previous document; drop it.
      app.sprite.events:off(watching)
      watching = nil
    end
    if app.sprite then
      watching = app.sprite.events:on("change", function(change)
        if not prefs.countEdits then return end
        if change.fromUndo then
          counts.undos = counts.undos + 1
        else
          counts.edits = counts.edits + 1
          prefs.totalEdits = prefs.totalEdits + 1
        end
      end)
    end
  end)

  local function colorChanged() counts.colors = counts.colors + 1 end
  app.events:on("fgcolorchange", colorChanged)
  app.events:on("bgcolorchange", colorChanged)

  plugin:newCommand{
    id = "event_report",
    title = "Event Report...",
    group = "file_scripts",
    onclick = function()
      local dlg = Dialog("Event Report")
      dlg:label{ text = ("Site changes: %d"):format(counts.sites) }:newrow()
         :label{ text = ("Color changes: %d"):format(counts.colors) }:newrow()
         :label{ text = ("Edits: %d, undo/redo: %d"):format(counts.edits, counts.undos) }:newrow()
         :label{ text = ("Edits over all sessions: %d (session %d)"):format(prefs.totalEdits, prefs.sessions) }
         :button{ id = "ok", text = "&OK", focus = true }
         :show()
    end,
  }

  plugin:newCommand{
    id = "count_edits",
    title = "Count Edits",
    group = "file_scripts",
    onchecked = function() return prefs.countEdits end,
    onclick = function() prefs.countEdits = not prefs.countEdits end,
  }
end

function exit(plugin)
  -- plugin.preferences is written after this returns.
  plugin.preferences.lastExit = counts.sites
end
