use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::AnalysisContext;

pub(crate) fn run_list(ctx: &AnalysisContext) {
    log::info!("Supported consoles:");
    log::info!("");

    let mut current_manufacturer = "";

    for console in ctx.consoles() {
        if console.metadata.manufacturer != current_manufacturer {
            if !current_manufacturer.is_empty() {
                log::info!("");
            }
            current_manufacturer = console.metadata.manufacturer;
            log::info!(
                "{}:",
                current_manufacturer.if_supports_color(Stdout, |t| t.bold()),
            );
        }

        let extensions = console.metadata.extensions.join(", ");
        let folders = console.metadata.folder_names.join(", ");
        let has_dat = console.analyzer.has_dat_support();

        log::info!(
            "  {} [{}]{}",
            console
                .metadata
                .short_name
                .if_supports_color(Stdout, |t| t.bold()),
            console
                .metadata
                .platform_name
                .if_supports_color(Stdout, |t| t.cyan()),
            if has_dat {
                format!(" {}", "(DAT)".if_supports_color(Stdout, |t| t.green()))
            } else {
                String::new()
            },
        );
        log::info!("    Extensions: {}", extensions);
        log::info!("    Folder names: {}", folders);
    }
}
