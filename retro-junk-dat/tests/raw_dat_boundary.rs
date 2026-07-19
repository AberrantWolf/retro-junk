use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.file_name().is_none_or(|n| n != "target") {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn raw_dat_loading_is_confined_to_catalog_imports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    rust_files(root, &mut files);
    let allowed = [
        "retro-junk-cli/src/commands/catalog/import.rs",
        "retro-junk-gui/src/backend/catalog_ops.rs",
        "retro-junk-dat/src/cache.rs",
    ];
    let loader_call = ["cache::", "load_dats"].concat();
    let offenders: Vec<_> = files
        .into_iter()
        .filter(|path| std::fs::read_to_string(path).is_ok_and(|s| s.contains(&loader_call)))
        .filter_map(|path| path.strip_prefix(root).ok().map(Path::to_path_buf))
        .filter(|path| !allowed.iter().any(|allowed| path == Path::new(allowed)))
        .collect();
    assert!(
        offenders.is_empty(),
        "runtime raw DAT loaders: {offenders:?}"
    );
}
