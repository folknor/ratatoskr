use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use squeeze::config::Config;
use squeeze::detect;

#[derive(Parser)]
#[command(name = "squeeze", about = "Compress email attachments transparently")]
struct Cli {
    /// Input file to compress.
    file: PathBuf,

    /// Output file path. Defaults to in-place with .orig backup.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// JPEG quality (0-100).
    #[arg(short, long, default_value_t = 80)]
    quality: u8,

    /// Maximum longest edge in pixels.
    #[arg(short = 'd', long, default_value_t = 2048)]
    max_dim: u32,

    /// Report savings without writing any files.
    #[arg(long)]
    dry_run: bool,

    /// Override MIME type detection.
    #[arg(long)]
    mime: Option<String>,

    /// Print detailed compression info.
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let input = match fs::read(&cli.file) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("error: failed to read {}: {e}", cli.file.display());
            return ExitCode::FAILURE;
        }
    };

    let mut config = Config::email_default();
    config.jpeg_quality = cli.quality;
    config.max_dimension = cli.max_dim;
    config.pdf_image_quality = cli.quality.min(75); // PDF images get slightly more aggressive.

    // Determine format.
    let mime_type = cli.mime.unwrap_or_default();
    let format = if mime_type.is_empty() {
        // Use file extension.
        let ext = cli
            .file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        detect::detect_from_extension(ext, &input)
    } else {
        detect::detect(&mime_type, &input)
    };

    if cli.verbose {
        eprintln!(
            "input: {} ({} bytes, format: {format:?})",
            cli.file.display(),
            input.len()
        );
    }

    if format == detect::Format::Unsupported {
        if cli.verbose {
            eprintln!("unsupported format, passing through unchanged");
        }
        return ExitCode::SUCCESS;
    }

    // Use format-appropriate MIME type so compress() doesn't re-detect wrong.
    let effective_mime = if mime_type.is_empty() {
        format.to_mime_type()
    } else {
        &mime_type
    };

    let result = match squeeze::compress(&input, effective_mime, &config) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if cli.verbose || cli.dry_run {
        eprintln!(
            "original:   {} bytes",
            result.original_size
        );
        eprintln!(
            "compressed: {} bytes",
            result.compressed_size
        );
        eprintln!("savings:    {:.1}%", result.savings_pct());
        if let Some(ref mime) = result.new_mime_type {
            eprintln!("new type:   {mime}");
        }
    }

    if !result.was_compressed() {
        if cli.verbose {
            eprintln!("no worthwhile savings, file unchanged");
        }
        return ExitCode::SUCCESS;
    }

    if cli.dry_run {
        return ExitCode::SUCCESS;
    }

    // Write output.
    let output_path = cli.output.unwrap_or_else(|| cli.file.clone());

    // If writing in-place, create .orig backup.
    if output_path == cli.file {
        let mut backup = cli.file.clone();
        let mut ext = cli
            .file
            .extension()
            .unwrap_or_default()
            .to_os_string();
        ext.push(".orig");
        backup.set_extension(ext);
        if let Err(e) = fs::rename(&cli.file, &backup) {
            eprintln!(
                "error: failed to create backup {}: {e}",
                backup.display()
            );
            return ExitCode::FAILURE;
        }
        if cli.verbose {
            eprintln!("backup: {}", backup.display());
        }
    }

    let bytes = result.into_bytes(&input);
    if let Err(e) = fs::write(&output_path, &bytes) {
        eprintln!(
            "error: failed to write {}: {e}",
            output_path.display()
        );
        return ExitCode::FAILURE;
    }

    if cli.verbose {
        eprintln!("wrote: {}", output_path.display());
    }

    ExitCode::SUCCESS
}
