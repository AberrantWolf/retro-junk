use super::*;

/// The starter template must parse as a valid config file, and — because
/// every key is commented out — must not make any field read as "set in
/// config file" during provenance checks.
#[test]
fn config_template_parses_with_no_values_set() {
    let config: ConfigFile = toml::from_str(CONFIG_TEMPLATE).expect("template must be valid TOML");
    let ss = config
        .screenscraper
        .expect("[screenscraper] section must be present");
    assert!(ss.dev_id.is_none());
    assert!(ss.dev_password.is_none());
    assert!(ss.soft_name.is_none());
    assert!(ss.user_id.is_none());
    assert!(ss.user_password.is_none());
}

/// The template must mention every field key and env var, so a user editing
/// it sees the full set of options.
#[test]
fn config_template_documents_every_field() {
    for meta in &CREDENTIAL_FIELDS {
        assert!(
            CONFIG_TEMPLATE.contains(meta.key),
            "template is missing key `{}`",
            meta.key
        );
        assert!(
            CONFIG_TEMPLATE.contains(meta.env_var),
            "template is missing env var `{}`",
            meta.env_var
        );
    }
}

/// A blank value (empty or whitespace-only) is "unset": it must fall through
/// to the next level of the chain rather than masking it. A blank `user_id`
/// means anonymous API access, not a login with an empty username.
#[test]
fn blank_values_are_treated_as_unset() {
    // non_blank filters empties and whitespace, keeps real values as-is
    assert_eq!(non_blank(None), None);
    assert_eq!(non_blank(Some(String::new())), None);
    assert_eq!(non_blank(Some("   ".to_string())), None);
    assert_eq!(non_blank(Some("abc".to_string())), Some("abc".to_string()));

    // Blank env value falls through to the file value
    assert_eq!(
        source_from(
            "VAR",
            Some(String::new()),
            Some("from-file"),
            CredentialSource::Missing,
        ),
        CredentialSource::ConfigFile
    );
    // Blank file value falls through to the fallback
    assert_eq!(
        source_from("VAR", None, Some("  "), CredentialSource::Embedded),
        CredentialSource::Embedded
    );
    assert_eq!(
        source_from("VAR", None, Some(""), CredentialSource::Missing),
        CredentialSource::Missing
    );
    // Non-blank env wins over everything
    assert_eq!(
        source_from(
            "VAR",
            Some("x".to_string()),
            Some("from-file"),
            CredentialSource::Missing,
        ),
        CredentialSource::EnvVar("VAR")
    );
}

#[test]
fn sources_by_key_covers_every_field() {
    let sources = CredentialSources {
        dev_id: CredentialSource::Missing,
        dev_password: CredentialSource::Missing,
        soft_name: CredentialSource::Default,
        user_id: CredentialSource::Missing,
        user_password: CredentialSource::Missing,
    };
    for meta in &CREDENTIAL_FIELDS {
        assert!(
            sources.by_key(meta.key).is_some(),
            "by_key must resolve `{}`",
            meta.key
        );
    }
    assert!(sources.by_key("nonsense").is_none());
}
