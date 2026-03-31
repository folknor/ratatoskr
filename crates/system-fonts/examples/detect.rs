use system_fonts::SystemFonts;

#[tokio::main]
async fn main() {
    let fonts = SystemFonts::detect().await;
    println!("System fonts detected:");
    println!("  UI:        {fonts_ui}", fonts_ui = format_font(&fonts.ui));
    println!(
        "  Monospace: {fonts_mono}",
        fonts_mono = format_font(&fonts.monospace)
    );
    println!(
        "  Document:  {fonts_doc}",
        fonts_doc = format_font(&fonts.document)
    );
}

fn format_font(font: &Option<system_fonts::SystemFont>) -> String {
    match font {
        Some(f) => match f.size {
            Some(size) => format!("{} {}pt", f.family, size),
            None => f.family.clone(),
        },
        None => "(not detected)".to_string(),
    }
}
