# Image formats: WebP, JPEG, BMP, TGA, ICO, PCX, FLI/FLC

Pixmith opens and writes these formats next to PNG and GIF. The code is `core/src/formats/raster/` (one module per
format, `abi.rs` for the C functions); the C declarations are in `app/Sources/PixmithCore/pixmith_formats.h`.

| Format | Open | Export | Notes |
| --- | --- | --- | --- |
| WebP | still and animated, lossy and lossless | lossless (default) or lossy with a quality; one frame or an animation | libwebp, built from the bundled sources of the `webp-animation` crate (nothing to install). Files larger than 3840 x 2160 pixels cannot be opened. |
| JPEG | baseline and progressive, gray and color | quality 1 to 100, 4:4:4 sampling | Flattened onto the background color. Rows come out the way zune-jpeg decodes them; EXIF orientation is not applied. |
| BMP | 1, 4, 8, 16, 24, 32 bit, RLE4 and RLE8, bit fields, top-down, OS/2 headers | 32 bit with alpha (BITMAPV5HEADER), 8 bit palette, 24 bit | Automatic depth: alpha gives 32, up to 256 colors gives 8, else 24. |
| TGA | color-mapped, true-color, gray, plain or RLE, any origin | 8 bit color-mapped, 24 bit, 32 bit with alpha, RLE optional | TGA 2.0 footer written. |
| ICO | any entry (BMP or PNG inside); cursors (.cur) read too | the sizes you pick, 1 to 256 | Open takes the largest size, or the one named (`ico_size`). Export scales the sprite by a whole number to fit each square and centers it (reduces it when it is larger); 256 is a PNG, smaller sizes are 32 bit bitmaps with an AND mask. No sizes: one image of the sprite's own size. |
| PCX | 1, 2, 4, 8 bit (packed or planar), 24 bit | 8 bit with a 256 color palette, 24 bit | No alpha: flattened onto the background. |
| FLI/FLC | FLI and FLC: COLOR_256, COLOR_64, DELTA_FLC, DELTA_FLI, BRUN, COPY, BLACK | FLC: palette chunk, BRUN or COPY first frame, DELTA_FLC frames, ring frame | 256 colors (median cut when there are more), no alpha. Each frame keeps its duration (frame header delay in milliseconds, plus the header speed). |

Opening: the sprite has one layer and one frame per image. A palette stored in the file becomes the sprite's palette;
the sprite stays in RGBA mode. Everything is bounded: dimensions, frame counts and memory are checked against the
header before anything large is allocated, and damaged files are errors (the decoders also run under `catch_unwind`).

## C functions

```c
PixmithDoc *pixmith_import_image(const char *path);                         // format from the content
PixmithDoc *pixmith_import_image_ex(const char *path, const char *json);    // {"ico_size": 32} or {"mode": "trace_grid", ...}
char *pixmith_image_info(const char *path);   // {"format":"ico","name":"ICO","animated":false,"sizes":[16,32,256]}
bool pixmith_export_image(const PixmithDoc *, uint32_t frame, uint32_t scale, const char *path, const char *json);
int32_t pixmith_export_run_ex(const PixmithDoc *, const PixmithExport *, const char *json, const char *path);
```

`pixmith_export_run_ex` is `pixmith_export_run` (tags, layers, trim, scale, one file per layer or tag) with the format
numbers 5 WebP, 6 JPEG, 7 BMP, 8 TGA, 9 ICO, 10 PCX, 11 FLC, plus the options JSON. WebP and FLC write one animation per
job, the others write `PixmithExport.frame`.

Export options (JSON, all optional): `quality`, `jpeg_quality`, `webp_quality`, `lossless`, `animated`, `background`
(`"#rrggbb"`), `bits` (0, 8, 24, 32), `rle`, `ico_sizes` (up to 8 sizes from 1 to 256).

## Command line

```sh
pixmith-cli export hero.pxmt --webp hero.webp [--lossy --quality 80] [--no-animation]
pixmith-cli export hero.pxmt --jpg hero.jpg --quality 90 --background '#202020'
pixmith-cli export hero.pxmt --bmp a.bmp --tga a.tga --pcx a.pcx --flc a.flc --bits 24
pixmith-cli export hero.pxmt --ico hero.ico --ico-sizes 16,32,48,256
pixmith-cli convert in.<any supported format> out.pxmt      # also out.webp, out.jpg, ... (first frame, or animation)
```

Any command that reads a document (`info`, `export`, `convert`) also accepts these image files.

## Front ends

- macOS: File > Open, drag and drop and File > Import > Image take the formats (Info.plist lists them as viewer
  types); File > Export… has them in its format popup with their options, File > Export As jumps to one. An icon with
  several sizes asks which size to open.
- egui: Open, drag and drop, File > Import > Image, the Export menu (Other formats) and File > Export As, with the same
  options and the same icon question.
