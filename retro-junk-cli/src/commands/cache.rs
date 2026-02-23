use std::fs;

use owo_colors::OwoColorize;
use owo_colors::Stream::Stdout;

use retro_junk_lib::AnalysisContext;

use super::analyze::format_bytes;

/// List cached DAT files.
pub(crate) fn run_cache_list() {
    match retro_junk_dat::cache::list() {
        Ok(entries) => {
            if entries.is_empty() {
                log::info!(
                    "{}",
                    "No cached DAT files.".if_supports_color(Stdout, |t| t.dimmed()),
                );
                log::info!("Run 'retro-junk cache fetch <system>' to download DAT files.");
                return;
            }

            log::info!(
                "{}",
                "Cached DAT files:".if_supports_color(Stdout, |t| t.bold()),
            );
            log::info!("");

            let mut total_size = 0u64;
            for entry in &entries {
                total_size += entry.file_size;
                log::info!(
                    "  {} [{}]",
                    entry.short_name.if_supports_color(Stdout, |t| t.bold()),
                    entry.dat_name.if_supports_color(Stdout, |t| t.cyan()),
                );
                log::info!(
                    "    Size: {}, Downloaded: {}, Version: {}",
                    format_bytes(entry.file_size),
                    entry.downloaded,
                    entry.dat_version,
                );
            }
            log::info!("");
            log::info!(
                "Total: {} files, {}",
                entries.len(),
                format_bytes(total_size)
            );
        }
        Err(e) => {
            log::warn!(
                "{} Error listing cache: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
        }
    }
}

/// Clear the DAT cache.
pub(crate) fn run_cache_clear() {
    match retro_junk_dat::cache::clear() {
        Ok(freed) => {
            log::info!(
                "{} Cache cleared ({} freed)",
                "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                format_bytes(freed),
            );
        }
        Err(e) => {
            log::warn!(
                "{} Error clearing cache: {}",
                "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                e,
            );
        }
    }
}

/// Fetch DAT files for specified systems.
pub(crate) fn run_cache_fetch(ctx: &AnalysisContext, systems: Vec<String>) {
    use retro_junk_lib::DatSource;

    let to_fetch: Vec<(String, Vec<&str>, &'static [&'static str], DatSource)> =
        if systems.len() == 1 && systems[0].eq_ignore_ascii_case("all") {
            ctx.consoles()
                .filter(|c| c.analyzer.has_dat_support())
                .map(|c| {
                    (
                        c.metadata.short_name.to_string(),
                        c.analyzer.dat_names().to_vec(),
                        c.analyzer.dat_download_ids(),
                        c.analyzer.dat_source(),
                    )
                })
                .collect()
        } else {
            systems
                .into_iter()
                .filter_map(|short_name| {
                    let console = ctx.get_by_short_name(&short_name);
                    match console {
                        Some(c) => {
                            let dat_names = c.analyzer.dat_names();
                            if dat_names.is_empty() {
                                log::warn!(
                                    "  {} No DAT support for '{}'",
                                    "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                                    short_name,
                                );
                                None
                            } else {
                                Some((
                                    short_name,
                                    dat_names.to_vec(),
                                    c.analyzer.dat_download_ids(),
                                    c.analyzer.dat_source(),
                                ))
                            }
                        }
                        None => {
                            log::warn!(
                                "  {} Unknown system '{}'",
                                "\u{26A0}".if_supports_color(Stdout, |t| t.yellow()),
                                short_name,
                            );
                            None
                        }
                    }
                })
                .collect()
        };

    for (short_name, dat_names, download_ids, dat_source) in &to_fetch {
        match retro_junk_dat::cache::fetch(short_name, dat_names, download_ids, *dat_source) {
            Ok(paths) => {
                let total_size: u64 = paths
                    .iter()
                    .filter_map(|p| fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum();
                if paths.len() == 1 {
                    log::info!(
                        "  {} {} ({})",
                        "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                        short_name.if_supports_color(Stdout, |t| t.bold()),
                        format_bytes(total_size),
                    );
                } else {
                    log::info!(
                        "  {} {} ({} DATs, {})",
                        "\u{2714}".if_supports_color(Stdout, |t| t.green()),
                        short_name.if_supports_color(Stdout, |t| t.bold()),
                        paths.len(),
                        format_bytes(total_size),
                    );
                }
            }
            Err(e) => {
                log::warn!(
                    "  {} {}: {}",
                    "\u{2718}".if_supports_color(Stdout, |t| t.red()),
                    short_name.if_supports_color(Stdout, |t| t.bold()),
                    e,
                );
            }
        }
    }
}
