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

- **Images**: JPEG (mozjpeg-rs, progressive + trellis), PNG (oxipng lossless), WebP, GIF, BMP, TIFF. HEIC is scaffolded behind an optional `heic` feature flag but not yet implemented.
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
  --estimate             Fast size prediction without compressing
  --mime <type>          Override MIME detection
  -v, --verbose          Detailed info
```

```bash
# Check what you'd save
squeeze --dry-run -v photo.jpg

# Instant size prediction (no CPU-intensive compression)
squeeze --estimate -v large_document.pdf

# Compress in place (creates photo.jpg.orig backup)
squeeze photo.jpg

# Compress to a different path
squeeze photo.jpg -o photo_small.jpg
```

## Library

Two entry points: `estimate()` for instant size prediction, `compress()` for actual compression.

### Estimating size

Two variants: `estimate()` for in-memory data, `estimate_file()` for file paths.

`estimate_file()` avoids loading the entire file for images (reads only headers, ~64 bytes) and ZIP archives (reads only the central directory). For PDFs, lopdf loads the full document into memory to walk the object graph. Sub-millisecond for images and archives, ~100-300ms for large PDFs.

```rust
use squeeze::{estimate::{estimate, estimate_file}, detect, config::Config};
use std::path::Path;

let config = Config::email_default();

// From a file path (preferred for images/archives — reads headers only):
let format = detect::detect_from_extension("pdf", &[]);
let est = estimate_file(Path::new("report.pdf"), format, &config)?;

// From memory (when you already have the bytes):
let format = detect::detect("image/jpeg", &file_bytes);
let est = estimate(&file_bytes, format, &config)?;

// est.expected_bytes  — conservative upper bound on compressed size
// est.floor_bytes     — non-compressible content (the hard minimum)
// est.worth_trying    — false if compression can't meaningfully help
```

The estimate is deliberately conservative — it over-predicts output size by 1-5x so that "won't fit" is a reliable signal. If `floor_bytes` exceeds the provider limit, no amount of compression will make the file small enough.

#### Typical integration: running total per email

```rust
use squeeze::{estimate::estimate_file, config::{Config, limits}, detect};
use std::path::Path;

let config = Config::email_default();
let provider_limit = limits::GMAIL_YAHOO; // 18 MB

let mut total_estimated: u64 = 0;

// User drops an attachment — estimate is fast, header-only for images/archives:
fn on_attachment_added(path: &Path, mime_type: &str) {
    let format = detect::detect(mime_type, &[]);
    let est = estimate_file(path, format, &config).unwrap();

    total_estimated += est.expected_bytes;

    if est.floor_bytes as usize > provider_limit {
        // This single file can never fit — warn immediately.
        // "This 220 MB PDF has 156 MB of non-image content and
        //  cannot be compressed below the 18 MB limit."
    } else if total_estimated as usize > provider_limit {
        // Combined attachments exceed limit — warn.
        // "Total attachments (~X MB) exceed the 18 MB limit.
        //  Remove an attachment or reduce quality."
    } else {
        // Looks good — compress in background.
        // Re-check total after compression completes
        // (actual size will be smaller than estimate).
    }
}
```

### Compressing

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

### Estimate vs compress

| | `estimate_file()` | `estimate()` | `compress()` |
|--|--|--|--|
| Input | File path | `&[u8]` | `&[u8]` |
| Memory | Headers only (images/archives), full file (PDFs) | Full buffer (but no work) | Full buffer + decode/encode |
| Speed | 2-80ms | Sub-millisecond | Seconds (CPU-intensive) |
| Accuracy | Conservative (1-5x over) | Conservative (1-5x over) | Exact |
| Use case | Pre-flight check | When bytes already in memory | Actual compression |

The intended flow: `estimate_file` when the user drops a file (instant, low memory), reject obvious failures, then load and `compress` in background for files that might fit.

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
- `quick-xml` 0.37 -- SVG parsing
- `clap` 4 -- CLI argument parsing

## Integration with Ratatoskr

Designed as a standalone crate for now. Later integration as optional dependency:

```toml
# In rtsk/Cargo.toml
squeeze = { path = "../../squeeze", optional = true }

[features]
compress-attachments = ["squeeze"]
```

The orchestration layer (running totals, provider limits, UI warnings) belongs in rtsk. Squeeze is a per-file tool — it doesn't know about "the email" or how many attachments there are.
