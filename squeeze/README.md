# squeeze

Transparent email attachment compression. Standalone Rust crate (CLI + library) that shrinks images, PDFs, and office documents before they leave the email client.

No major email client does this well. Outlook can resize images. Apple Mail prompts for quality. Nobody does transparent cross-format compression.

## Results

Real-world test fixtures, `Config::email_default()` (JPEG q80, max 2048px, PDF images q75 max 1500px):

| File | Type | Original | Compressed | Savings |
|------|------|----------|------------|---------|
| Phone photo | JPEG | 16.8 MB | 519 KB | **96.9%** |
| Phone photo | JPEG | 12.7 MB | 197 KB | **98.4%** |
| Phone photo | JPEG | 12.4 MB | 548 KB | **95.6%** |
| Scanned document | BMP | 104 MB | 60 KB | **99.9%** |
| Scanned document | BMP | 26 MB | 216 KB | **99.2%** |
| Screenshot | PNG | 28 MB | 8.8 MB | **69.0%** |
| Screenshot | PNG | 7.2 MB | 5.7 MB | **20.7%** |
| Old photo | TIFF | 28 MB | 223 KB | **99.2%** |
| Animated GIF | GIF | 3.9 MB | 95 KB | **97.5%** |
| Image-heavy PDF | PDF | 6.8 MB | 499 KB | **92.7%** |
| Image-heavy PDF | PDF | 30 MB | 2.1 MB | **93.2%** |
| Mixed PDF (1775 pages) | PDF | 220 MB | 65 MB | **70.5%** |
| Scanned PDF | PDF | 37 MB | 19 MB | **47.8%** |
| Presentation (slides) | PPTX | 13.6 MB | 5.2 MB | **61.3%** |
| Document with images | ODT | 3.2 MB | 338 KB | **89.5%** |
| Inkscape SVG (17 embedded images) | SVG | 114 MB | 86 MB | **24.9%** |
| Small icon | JPEG | 82 KB | 65 KB | **20.5%** |
| Small PNG | PNG | 780 KB | 667 KB | **14.4%** |

Files already small or without compressible content pass through unchanged.

## Supported formats

- **Images**: JPEG (mozjpeg-rs, progressive + trellis), PNG (oxipng lossless), WebP, GIF, BMP, TIFF. HEIC behind optional `heic` feature flag.
- **PDFs**: DCTDecode (JPEG) and FlateDecode (raw pixel) image recompression, stream deduplication, unused object removal, metadata stripping, PDF 1.5 object streams (`save_modern`).
- **SVGs**: Strip editor metadata (Inkscape/Sodipodi), optimize embedded base64 PNG/JPEG images.
- **OOXML**: .docx, .xlsx, .pptx, .docm, .xlsm, .pptm -- compresses images in `word/media/`, `xl/media/`, `ppt/media/`.
- **ODF**: .odt, .ods, .odp -- compresses images in `Pictures/`.

## CLI

```
squeeze <file> [OPTIONS]

Options:
  -o, --output <path>    Output path (default: in-place with .orig backup)
  -q, --quality <0-100>  JPEG quality [default: 80]
  -d, --max-dim <px>     Max longest edge [default: 2048]
  --dry-run              Report savings without writing
  --mime <type>          Override MIME detection
  -v, --verbose          Detailed info
```

```bash
# Check what you'd save
squeeze --dry-run -v photo.jpg

# Compress in place (creates photo.jpg.orig backup)
squeeze photo.jpg

# Compress to a different path
squeeze photo.jpg -o photo_small.jpg
```

## Library

```rust
use squeeze::{compress, config::Config};

let config = Config::email_default();
let result = compress(&file_bytes, "image/jpeg", &config)?;

if result.was_compressed() {
    println!("saved {:.1}%", result.savings_pct());
    let output = result.into_bytes(&file_bytes);
    // use output...
}
```

## Provider size limits

After MIME base64 encoding (~37% overhead), actual file size limits:

| Provider | Stated limit | Effective file limit |
|----------|-------------|---------------------|
| Exchange (strict) | 10 MB | ~7 MB |
| Outlook.com / iCloud | 20 MB | ~15 MB |
| Gmail / Yahoo / Proton | 25 MB | ~18 MB |

Available as `squeeze::config::limits::{EXCHANGE_STRICT, OUTLOOK_ICLOUD, GMAIL_YAHOO}`.

## Dependencies

All pure Rust except optional HEIC:

- `image` 0.25 -- decoding all image formats, resizing
- `mozjpeg-rs` 0.9 -- JPEG encoding (pure Rust mozjpeg reimplementation)
- `oxipng` 10 -- lossless PNG optimization
- `lopdf` 0.39 -- PDF manipulation
- `zip` 2 -- OOXML/ODF archive handling
- `clap` 4 -- CLI argument parsing

## Integration with Ratatoskr

Designed as a standalone crate for now. Later integration as optional dependency:

```toml
# In ratatoskr-core/Cargo.toml
squeeze = { path = "../../squeeze", optional = true }

[features]
compress-attachments = ["squeeze"]
```
