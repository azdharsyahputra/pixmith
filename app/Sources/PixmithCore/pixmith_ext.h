// Extensions: .pixmith-extension packages that contribute scripts, palettes and themes (docs/EXTENSIONS.md).
// Results are JSON text; release them with pixmith_string_free. Failing calls set pixmith_last_error().

#ifndef PIXMITH_EXT_H
#define PIXMITH_EXT_H

#include <stdbool.h>

// <app data>/Extensions (honors PIXMITH_SUPPORT_DIR). The folder may not exist yet.
char *pixmith_ext_dir(void);

// Reads the folder again: call when Preferences opens and after changes made outside the app.
void pixmith_ext_refresh(void);

// JSON array of installed extensions:
// {name, displayName, version, author, description, enabled, dir, error|null,
//  scripts:[{name,path}], palettes:[{name,colors}], themes:[{id,name,dark,light}]}
char *pixmith_ext_list(void);

// Validates and installs (or updates) a package. Returns the extension as JSON, or NULL with the reason in
// pixmith_last_error(). A rejected package leaves nothing behind.
char *pixmith_ext_install(const char *path);

bool pixmith_ext_set_enabled(const char *name, bool enabled);
bool pixmith_ext_uninstall(const char *name);

// Themes of the enabled extensions: [{id, name, extension, dark:{token:"RRGGBBAA"}, light:{...}}].
char *pixmith_ext_themes(void);

// Scripts of the enabled extensions: [{extension, name, path, dir, prefs}] (dir: the extension folder; prefs: the file for
// the plugin's preferences).
char *pixmith_ext_scripts(void);

#endif
