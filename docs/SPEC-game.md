# Pixmith — Spesifikasi Fitur

Pixmith = editor pixel art untuk bikin game. Tiga fitur pembeda utama:

1. Export Godot native (SpriteFrames + TileSet)
2. Auto-tiling
3. Tweening

Semua logika ditaruh di Rust core (`pixmith-core`), misalnya `export/godot`, `tilemap/autotile`, `anim/tween`. UI Mac (SwiftUI) dan Windows (egui) tinggal manggil.

Bagian **⚠️ Critical** = hal yang paling sering bikin bug atau bikin user kesel kalau salah.

> Catatan: format `.tres` di bawah ditulis berdasarkan pengetahuan Godot 4, belum diverifikasi ke versi tertentu. Sebelum implementasi, bikin SpriteFrames dan TileSet kecil manual di Godot versi target, save, lalu bandingkan isinya.

---

## 1. Export Godot native (target: Godot 4)

### 1A. SpriteFrames (`.tres`) untuk animasi

**Output:**

```
player.png      ← spritesheet
player.tres     ← SpriteFrames, merujuk ke player.png
```

**Struktur file** (format teks Godot 4, `format=3`):

```
[gd_resource type="SpriteFrames" format=3]

[ext_resource type="Texture2D" path="res://art/player.png" id="1_tex"]

[sub_resource type="AtlasTexture" id="at_idle_0"]
atlas = ExtResource("1_tex")
region = Rect2(0, 0, 16, 16)

[sub_resource type="AtlasTexture" id="at_idle_1"]
atlas = ExtResource("1_tex")
region = Rect2(16, 0, 16, 16)

[resource]
animations = [{
"frames": [{"duration": 1.0, "texture": SubResource("at_idle_0")},
           {"duration": 2.0, "texture": SubResource("at_idle_1")}],
"loop": true,
"name": &"idle",
"speed": 10.0
}]
```

**Mapping Pixmith → Godot:**

| Pixmith | Godot |
|---|---|
| Tag animasi | 1 entry di `animations` (`name`) |
| Frame di tag | 1 `AtlasTexture` + entry di `frames` |
| Durasi frame (ms) | `speed` (FPS) + `duration` (pengali relatif) |
| Tag loop / sekali jalan | `loop: true/false` |
| Layer tersembunyi | tidak ikut di-export |

**⚠️ Critical:**

1. **Konversi durasi.** Godot tidak menyimpan milidetik. Durasi asli frame = `duration / speed`. Kalau durasi frame berbeda-beda (100ms, 150ms, 50ms), cari FPB semua durasi (50ms), lalu `speed = 1000/50 = 20` dan `duration = ms/50` (2, 3, 1). Salah konversi = timing animasi di game meleset.
2. **Ping-pong tidak ada di Godot.** Tag dengan arah ping-pong atau reverse harus di-expand jadi urutan frame biasa: `0,1,2,3,2,1`. Frame ujung jangan dobel.
3. **Pixel art blur.** Default filter Godot = Linear. User harus set `Project Settings → Rendering → Textures → Default Texture Filter = Nearest` (atau `texture_filter` per node). Tampilkan peringatan ini saat export pertama.
4. **Trim butuh offset.** Kalau frame di-trim untuk menghemat atlas, set `margin = Rect2(...)` di `AtlasTexture` supaya ukuran dan posisi asli kembali. Tanpa ini sprite goyang tiap ganti frame.
5. **Texture bleeding.** Kamera di posisi subpixel bisa mengambil pixel frame sebelah. Sediakan opsi padding + extrude 1px di spritesheet.
6. **Re-export tidak boleh merusak project.** ID `sub_resource` harus stabil (misal `at_<tag>_<index>`), bukan acak. Kalau `.tres` sudah ada dan header-nya punya `uid="uid://..."`, baca dan pertahankan. Kalau berubah, referensi di scene Godot bisa rusak.
7. **Path `res://`.** User memilih root project Godot (folder yang berisi `project.godot`). Path dihitung relatif dari situ. Tolak export ke luar root.
8. **Batas ukuran texture.** Maksimal 16384px, aman di mobile 4096px. Kalau lewat, pecah atlas jadi beberapa halaman (beberapa `ext_resource`).
9. **Godot 3 berbeda format** (`format=2`, struktur lain). Fokus Godot 4 saja dan tulis jelas di UI.

### 1B. TileSet (`.tres`) untuk tilemap

```
[gd_resource type="TileSet" format=3]

[ext_resource type="Texture2D" path="res://art/tiles.png" id="1_tex"]

[sub_resource type="TileSetAtlasSource" id="src_0"]
texture = ExtResource("1_tex")
texture_region_size = Vector2i(16, 16)
0:0/0 = 0
0:0/0/terrain_set = 0
0:0/0/terrain = 0
0:0/0/terrains_peering_bit/right_side = 0
0:0/0/terrains_peering_bit/bottom_right_corner = 0
0:0/0/terrains_peering_bit/bottom_side = 0

[resource]
tile_size = Vector2i(16, 16)
terrain_set_0/mode = 0
terrain_set_0/terrain_0/name = "Grass"
terrain_set_0/terrain_0/color = Color(0.3, 0.7, 0.3, 1)
sources/0 = SubResource("src_0")
```

**Mapping:**

- `x:y/0` = tile di koordinat atlas (kolom:baris), bukan pixel.
- `terrain_set_N/mode`: `0` = Match Corners and Sides (blob 47), `1` = Match Corners, `2` = Match Sides (16 tile).
- `terrains_peering_bit/<arah>`: `top_side`, `top_right_corner`, `right_side`, `bottom_right_corner`, `bottom_side`, `bottom_left_corner`, `left_side`, `top_left_corner`.

**⚠️ Critical:**

1. Mode auto-tile Pixmith harus sama persis dengan mode terrain Godot. Kalau tidak, tile di game berbeda dengan di editor.
2. Collision (opsional, versi berikutnya): `physics_layer_0/...` plus polygon per tile. Koordinat relatif ke tengah tile, bukan pojok kiri atas.
3. Format `.tres` tidak punya schema resmi dan bisa berubah antar versi Godot. Wajib dites di Godot asli sebelum rilis.

### 1C. TileSet isometrik

Tileset isometrik (diamond 2:1, `core/src/document/iso.rs`) diekspor sebagai `tile_shape = 1` (ISOMETRIC) dan `tile_layout = 5` (DIAMOND_DOWN). `tile_size` adalah ukuran diamond (misal 96x48), `texture_region_size` ukuran gambar tile (misal 96x96 untuk tile yang lebih tinggi dari diamond). Sel `(i, j)` ada di `(x + (i - j) * w/2, y + (i + j) * h/2)`; posisi node `TileMapLayer` = offset grid layer (pojok kiri atas kotak sel (0, 0)), `y_sort_enabled = true`.

- **Pivot tile** = titik di gambar tile (dari pojok kiri atas) yang jatuh di tengah diamond; default `(w/2, h - cell_h/2)` untuk gambar lebih tinggi dari diamond. Godot: `texture_origin = pivot - ukuran/2` (gambar digambar di `tengah sel - ukuran/2 - texture_origin`), ditulis hanya kalau tidak nol.
- **Peering** mengikuti tetangga sel: N=(0,-1) `top_right_side`, NE=(1,-1) `right_corner`, E=(1,0) `bottom_right_side`, SE=(1,1) `bottom_corner`, S=(0,1) `bottom_left_side`, SW=(-1,1) `left_corner`, W=(-1,0) `top_left_side`, NW=(-1,-1) `top_corner`. Mode Sides memakai sisi, Blob = Match Corners and Sides. Dual-grid (Corners) belum ada di layer isometrik.
- Flip tile isometrik ditulis sebagai flag sel (bukan tile alternatif).
- Diverifikasi dengan Godot 4.7.2: hasil render scene sama persis per pixel dengan render Pixmith (lihat `write_godot_sample_project` di `gamedev/godot/tileset/iso_tests.rs`).

---

## 2. Auto-tiling

### Mode yang didukung

| Mode | Jumlah tile | Cara kerja | Catatan |
|---|---|---|---|
| Sides (4-bit) | 16 | cek 4 tetangga sisi | paling simpel, sudut kasar |
| Blob (8-bit) | 47 | cek 8 tetangga | paling umum, paling halus |
| Corners / dual-grid | 16 | nilai di sudut, bukan di sel | populer; cukup ~5–6 bentuk unik, sisanya rotasi |

### Bitmask blob

```
NW=128  N=1   NE=2
W=64    [X]   E=4
SW=32   S=16  SE=8
```

Aturan reduksi: bit sudut hanya dihitung kalau kedua sisi di sebelahnya juga terisi. Dari 256 kombinasi tersisa 47.

```rust
fn reduce(mut m: u8) -> u8 {
    const N: u8 = 1; const NE: u8 = 2; const E: u8 = 4; const SE: u8 = 8;
    const S: u8 = 16; const SW: u8 = 32; const W: u8 = 64; const NW: u8 = 128;
    if m & N == 0 || m & E == 0 { m &= !NE; }
    if m & S == 0 || m & E == 0 { m &= !SE; }
    if m & S == 0 || m & W == 0 { m &= !SW; }
    if m & N == 0 || m & W == 0 { m &= !NW; }
    m
}
```

### Model data

```rust
struct Tileset { tile_size: (u32, u32), tiles: Vec<Tile> }
struct Tile    { id: TileId, pixels: ImageRef, terrain: Option<TerrainId>,
                 mask: u8, weight: f32 }          // weight untuk variasi
struct TilemapLayer {
    size: (u32, u32),
    terrain: Grid<Option<TerrainId>>,   // yang dicat user (niat)
    resolved: Grid<Option<TileRef>>,    // hasil hitungan (cache)
    overrides: Grid<Option<TileRef>>,   // tile yang dipasang manual
}
struct TileRef { tile: TileId, flip_x: bool, flip_y: bool, rot90: u8 }
```

**⚠️ Critical:**

1. **Simpan niat (terrain), bukan hanya hasil (tile).** Kalau cuma tile hasil yang disimpan, menghapus satu sel membuat tetangga tidak bisa dihitung ulang dengan benar. `resolved` hanya cache dan harus bisa dibangun ulang kapan saja.
2. **Update lokal.** Saat satu sel dicat, hitung ulang sel itu plus 8 tetangga (3×3); dual-grid cukup 2×2. Jangan hitung ulang seluruh map tiap klik.
3. **Tile yang tidak ada.** User belum tentu menggambar lengkap 47 tile. Kalau mask tidak tersedia, pakai tile dengan bit cocok terbanyak dan tandai sel itu di UI. Jangan crash, jangan dibiarkan kosong.
4. **Variasi acak harus deterministik.** Pilih di antara tile bermask sama dengan `hash(x, y, seed)`, bukan `rand()`. Kalau tidak, tile berganti-ganti tiap redraw.
5. **Pinggir map.** Opsi: sel di luar map dianggap kosong atau terrain yang sama. Default: kosong.
6. **Override manual menang.** Auto-tile tidak boleh menimpa tile yang dipasang manual.
7. **Edit tile = semua instance ikut berubah.** Tile dirujuk lewat `TileId`, pixel tidak disalin.
8. **Multi-terrain** (rumput ↔ tanah ↔ air) kombinasinya meledak. Rilis pertama: satu terrain vs kosong saja.

---

## 3. Tweening

### Konsep

User membuat keyframe untuk properti layer di frame tertentu; frame di antaranya diisi otomatis.

```rust
struct TweenTrack { layer: LayerId, property: Prop, keys: Vec<Key> }
enum Prop { PosX, PosY, Rotation, ScaleX, ScaleY, Opacity }
struct Key { frame: u32, value: f32, ease: Ease }   // ease = kurva menuju key berikutnya
enum Ease { Linear, Hold, InQuad, OutQuad, InOutQuad, InOutCubic,
            OutBack, OutBounce, Bezier(f32, f32, f32, f32) }
struct Pivot { x: f32, y: f32 }   // boleh .5
```

### Alur render tiap frame

```
pixel sumber (tidak pernah diubah)
   → hitung transform di frame ini (interpolasi + easing)
   → transform dengan nearest neighbor / RotSprite
   → bulatkan posisi ke pixel
   → hasil (preview atau bake)
```

**⚠️ Critical:**

1. **Selalu render dari sumber asli.** Jangan transform hasil frame sebelumnya; transform berantai merusak pixel secara bertahap.
2. **Pembulatan konsisten.** Pakai satu cara di semua tempat, misal `floor(x + 0.5)`. Kalau dicampur, sprite jitter 1px.
3. **Pivot setengah pixel.** Pusat sprite 16×16 ada di (8, 8), yaitu pojok pixel. Rotasi di (7, 7) membuat miring sebelah. Pivot harus mendukung `.5`.
4. **Rotasi.**
   - Kelipatan 90° = lossless, putar array langsung.
   - Sudut lain = RotSprite (upscale 8× dengan Scale2x tiga kali, rotasi nearest, downsample). Lambat, jadi hasilnya di-cache.
   - Pilihan untuk user: RotSprite (rapi) atau nearest (cepat, kasar).
5. **Skala non-integer** (misal 1.5×) membuat ukuran pixel tidak rata. Beri peringatan atau opsi snap ke skala integer.
6. **Opacity menghasilkan warna baru.** Di mode palet terbatas (indexed), kuantisasi ke palet atau matikan opacity.
7. **Bake wajib ada.** Pixel artist hampir selalu merapikan hasil tween manual. Tombol "Bake" mengubah frame tween jadi pixel biasa; opsi "Bake & keep source" supaya bisa bake ulang.
8. **Berbasis frame atau waktu?** Rekomendasi: frame index. Lebih intuitif untuk pixel artist dan tidak terganggu durasi frame yang berbeda-beda.
9. **Export ke Godot.** Rilis pertama: frame hasil bake masuk SpriteFrames biasa. Nanti: opsi export sebagai track `AnimationPlayer`.

---

## Pengujian

- **Golden test:** simpan `.tres` dan `.png` yang benar, bandingkan byte per byte tiap build. Export harus deterministik (ID dan urutan stabil).
- **Godot headless di CI:** import project contoh, pastikan tidak ada error.
- **Autotile:** uji semua 256 mask; `reduce()` harus menghasilkan tepat 47 nilai unik.
- **Tween:** transform identitas harus menghasilkan output = input, pixel per pixel.

---

## Ringkasan risiko

| Fitur | Bencana kalau... |
|---|---|
| Export Godot | uid/ID tidak stabil, filter blur, durasi salah |
| Auto-tile | hanya menyimpan tile hasil, variasi pakai `rand()` |
| Tween | transform berantai, pivot salah, tidak ada tombol bake |

## Urutan pengerjaan

1. Export Godot SpriteFrames
2. Auto-tiling (satu terrain) + export TileSet
3. Tweening + bake
