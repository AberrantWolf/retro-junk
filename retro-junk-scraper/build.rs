use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("embedded_credentials.rs");

    let dev_id = env::var("SCREENSCRAPER_DEVID").ok();
    let dev_password = env::var("SCREENSCRAPER_DEVPASSWORD").ok();

    let key: &[u8] = b"rj-ss-obfuscation-9f3a7c1b";

    let mut code = String::new();
    code.push_str(&format!(
        "const OBFUSCATION_KEY: &[u8] = &{:?};\n\n",
        key
    ));

    // Only embed if BOTH are provided
    match (&dev_id, &dev_password) {
        (Some(id), Some(pw)) => {
            let encoded_id = xor_encode(id.as_bytes(), key);
            let encoded_pw = xor_encode(pw.as_bytes(), key);

            code.push_str(&format!(
                "const EMBEDDED_DEV_ID: Option<&[u8]> = Some(&{:?});\n",
                encoded_id.as_slice()
            ));
            code.push_str(&format!(
                "const EMBEDDED_DEV_PASSWORD: Option<&[u8]> = Some(&{:?});\n",
                encoded_pw.as_slice()
            ));
        }
        _ => {
            code.push_str("const EMBEDDED_DEV_ID: Option<&[u8]> = None;\n");
            code.push_str("const EMBEDDED_DEV_PASSWORD: Option<&[u8]> = None;\n");
        }
    }

    fs::write(&dest_path, code).unwrap();

    println!("cargo:rerun-if-env-changed=SCREENSCRAPER_DEVID");
    println!("cargo:rerun-if-env-changed=SCREENSCRAPER_DEVPASSWORD");
}

fn xor_encode(data: &[u8], key: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect()
}
