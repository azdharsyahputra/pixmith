// Pixmith documents (.pxmt) and projects (core/src/formats/pxmt.rs, core/src/project.rs).
//
// .pxmt is Pixmith's own document format and the default for Save. It stores everything a document holds
// (the same sprite model as .aseprite, plus Pixmith-only data such as tweens, terrains and export settings)
// and a small embedded thumbnail so project browsers never have to decode whole files. .aseprite / .ase stay
// supported: Open imports them, and Export As Aseprite writes them.
//
// pixmith_open() and pixmith_save() (pixmith.h) choose the format themselves: pixmith_open() reads .pxmt by its
// magic bytes and anything else as .aseprite; pixmith_save() writes .pxmt when the path ends in ".pxmt" and
// .aseprite otherwise.

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Writes the document as .aseprite regardless of the path's extension (File > Export As Aseprite…).
// Pixmith-only data that .aseprite cannot hold is kept where Aseprite allows (user data) or dropped.
bool pixmith_save_aseprite(const PixmithDoc *doc, const char *path);

// A preview of any document file (.pxmt: its embedded thumbnail; .aseprite/.ase/.png: frame 1 rendered),
// fitted inside max_w x max_h keeping the aspect ratio, nearest-neighbour, straight-alpha RGBA.
// `out` must hold max_w * max_h * 4 bytes; *w and *h receive the actual size. False if the file can't be read.
bool pixmith_file_thumbnail(const char *path, uint32_t max_w, uint32_t max_h, uint8_t *out, size_t len,
                            uint32_t *w, uint32_t *h);

// --- Projects ------------------------------------------------------------------------------------------
// A project is a folder: canvases (*.pxmt, and also *.aseprite / *.ase found there) plus a ".pixmith" folder
// holding project.json (name, created, canvas order) and the project's trash. Strings returned below are
// freed with pixmith_string_free; NULL / false means failure (see pixmith_last_error).

// Makes `folder` a project (creating the folder if needed) named `name`. Safe on an existing project.
bool pixmith_project_create(const char *folder, const char *name);

// True when `folder` is a project (has .pixmith/project.json).
bool pixmith_project_is_project(const char *folder);

// The project as JSON:
// {"name":"…","folder":"/abs","canvases":[{"path":"/abs/a.pxmt","name":"a","format":"pxmt"|"aseprite",
//   "width":32,"height":32,"frames":4,"layers":3,"modified":1729700000,"size":12345}, …]}
// Canvases follow the project's saved order, then new files by name. Unreadable files are listed with
// "error":"…" instead of the size fields.
char *pixmith_project_scan(const char *folder);

bool pixmith_project_set_name(const char *folder, const char *name);

// Saves the order of canvases (a JSON array of file names) shown by the project browser.
bool pixmith_project_set_order(const char *folder, const char *json_file_names);

// A new blank .pxmt canvas in the project (name made unique: "Canvas", "Canvas 2", …). Returns its path.
char *pixmith_project_new_canvas(const char *folder, const char *name, uint32_t width, uint32_t height);

// Copies a canvas next to itself ("hero copy.pxmt", …). Returns the new path.
char *pixmith_project_duplicate_canvas(const char *path);

// Renames a canvas file, keeping its extension; refuses an existing name. Returns the new path.
char *pixmith_project_rename_canvas(const char *path, const char *new_name);

// Moves a canvas into the project's .pixmith/trash (restorable). Returns the path inside the trash.
char *pixmith_project_trash_canvas(const char *path);

// Moves a trashed canvas back into the project. Returns its restored path.
char *pixmith_project_restore_canvas(const char *trashed_path);

// Copies an .aseprite/.ase/.png file into the project as a .pxmt canvas. Returns the new path.
char *pixmith_project_import(const char *folder, const char *file);
