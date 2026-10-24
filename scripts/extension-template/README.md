# Sample extension

A working extension to copy from: a palette, a color theme and a plugin command.

```
package.json            name, displayName, version, author, description, contributes
scripts/sample.lua      a plugin: init(plugin) registers a command, shown in File > Scripts
palettes/sunset.gpl     a GIMP palette (.gpl) or a list of hex colors (.hex)
themes/midnight.json    design token overrides for the dark and/or light appearance
```

Pack and install:

```sh
scripts/extension-template/pack.sh                 # writes sample-extension-1.0.0.pixmith-extension
```

Then **Preferences > Extensions > Add Extension...** (or open or drop the file on Pixmith). The palette appears
in the Palette presets menu, the theme in **Preferences > Appearance > Color theme**, and the command under
**File > Scripts**.

The full format (manifest fields, theme tokens, limits) is in [`docs/EXTENSIONS.md`](../../docs/EXTENSIONS.md).
