use super::*;
use crate::I18n;
use curlybrace_core::connection::{ConnectionColor, SshAuth, TlsConfig};
use curlybrace_core::secrets::{MemorySecretStore, SecretKind, secret_key};

fn valid() -> ConnectionForm {
    ConnectionForm {
        name: "Prod ☃ Überdatenbank".into(),
        uri: "mongodb://localhost:27017".into(),
        ..ConnectionForm::default()
    }
}

fn ssh_form() -> ConnectionForm {
    ConnectionForm {
        ssh_enabled: true,
        ssh_host: "bastion".into(),
        ssh_port: "22".into(),
        ssh_user: "me".into(),
        ..valid()
    }
}

#[test]
fn the_form_checks_the_host_list_strictly() {
    for ok in [
        "mongodb://h",
        "mongodb://h1:27017,h2:27018",
        "mongodb://[::1]:27017",
        "mongodb+srv://cluster.example.com",
        "mongodb://%2Ftmp%2Fmongodb-27017.sock",
    ] {
        let f = ConnectionForm {
            uri: ok.into(),
            ..valid()
        };
        assert!(f.to_checked_config().is_ok(), "{ok}");
    }
    for bad in ["mongodb://u:pa/ss@h", "mongodb://u@", "mongodb://h:0"] {
        let f = ConnectionForm {
            uri: bad.into(),
            ..valid()
        };
        let err = f.to_checked_config().unwrap_err();
        assert!(matches!(err, CoreError::InvalidUri(_)), "{bad}");
        assert!(!err.to_string().contains("pa/ss"), "{bad}: {err}");
    }
}

#[test]
fn save_points_a_unicode_host_to_from_uri_instead_of_rewriting_it() {
    let f = ConnectionForm {
        uri: "mongodb://bücher.example/".into(),
        ..valid()
    };
    let err = f.to_checked_config().unwrap_err();
    assert_eq!(err.i18n_key(), "errors.invalid_uri_non_ascii_host");
}

#[test]
fn save_rejects_an_at_sign_in_the_tail_or_userinfo_and_a_trailing_comma_without_echo() {
    for bad in [
        "mongodb://admin:2024/S3CR3T@db",
        "mongodb://u:12345?S3CR3T@h",
        "mongodb://u:pa@S3CR3T@h",
        "mongodb://h/db@x",
        "mongodb://u@x@h",
        "mongodb://h1:27017,h2:27018,",
    ] {
        let f = ConnectionForm {
            uri: bad.into(),
            ..valid()
        };
        let err = f.to_checked_config().unwrap_err();
        assert!(matches!(err, CoreError::InvalidUri(_)), "{bad}");
        assert!(!err.to_string().contains("S3CR3T"), "{bad}: {err}");
    }
}

#[test]
fn save_accepts_an_at_sign_in_a_key_value_query() {
    for ok in [
        "mongodb://h/?appName=bob@laptop",
        "mongodb://h/db?tlsCAFile=/certs@2/ca.pem",
    ] {
        let f = ConnectionForm {
            uri: ok.into(),
            ..valid()
        };
        assert!(f.to_checked_config().is_ok(), "{ok}");
    }
}

#[test]
fn test_and_copy_uri_of_a_stored_bad_host_connection_still_build_a_config() {
    for stored in ["mongodb://u:pa/ss@h", "mongodb://admin:2024/S3CR3T@db"] {
        let cfg = ConnectionConfig::new("n", stored);
        let form = ConnectionForm::from_config(&cfg);
        assert!(form.to_config().is_ok(), "{stored}");
        assert!(form.to_checked_config().is_err(), "{stored}");
    }
}

#[test]
fn apply_parsed_fills_fields_and_keeps_the_password_out_of_the_uri() {
    let parsed =
        curlybrace_core::uri::parse_uri("mongodb://bob:pw@h/db?authSource=admin&tls=true").unwrap();
    let form = ConnectionForm {
        name: "n".into(),
        clear_password: true,
        ..ConnectionForm::default()
    }
    .apply_parsed(parsed);
    assert_eq!(form.uri, "mongodb://h/db");
    assert_eq!(form.username, "bob");
    assert_eq!(form.password, "pw");
    assert_eq!(form.auth_source, "admin");
    assert!(form.tls_enabled);
    assert!(!form.clear_password);
    assert!(form.to_config().is_ok(), "no password in the uri");
}

#[test]
fn apply_parsed_without_a_password_keeps_what_the_user_typed() {
    let form = ConnectionForm {
        password: "typed".into(),
        clear_password: true,
        ..ConnectionForm::default()
    };
    let form = form.apply_parsed(curlybrace_core::uri::parse_uri("mongodb://bob@h").unwrap());
    assert_eq!(form.password, "typed");
    assert!(form.clear_password, "untouched when no password was parsed");
}

#[test]
fn valid_form_maps_to_config() {
    let f = ConnectionForm {
        auth: AuthMechanism::ScramSha256,
        username: "bob".into(),
        tls_enabled: true,
        tls_ca: "/tmp/ca.pem".into(),
        ..valid()
    };
    let c = f.to_config().unwrap();
    assert_eq!(c.id, f.id);
    assert_eq!(c.name, "Prod ☃ Überdatenbank");
    assert_eq!(c.username.as_deref(), Some("bob"));
    assert_eq!(
        c.tls,
        TlsConfig {
            enabled: true,
            ca_file: Some("/tmp/ca.pem".into()),
            ..TlsConfig::default()
        }
    );
    assert!(c.ssh.is_none());
}

#[test]
fn blank_name_and_bad_uri_are_rejected() {
    let f = ConnectionForm {
        name: "  ".into(),
        ..valid()
    };
    assert!(matches!(f.to_config(), Err(CoreError::InvalidName)));
    let f = ConnectionForm {
        uri: "nope".into(),
        ..valid()
    };
    assert!(matches!(f.to_config(), Err(CoreError::InvalidUri(_))));
}

#[test]
fn ssh_port_must_parse_and_fields_required() {
    let f = ConnectionForm {
        ssh_port: "abc".into(),
        ..ssh_form()
    };
    assert!(matches!(f.to_config(), Err(CoreError::Ssh(_))));
    let f = ConnectionForm {
        ssh_host: " ".into(),
        ..ssh_form()
    };
    assert!(f.to_config().is_err());
    let f = ConnectionForm {
        ssh_user: "".into(),
        ..ssh_form()
    };
    assert!(f.to_config().is_err());
    let f = ConnectionForm {
        ssh_port: "0".into(),
        ..ssh_form()
    };
    assert!(f.to_config().is_err());
}

#[test]
fn ssh_disabled_ignores_garbage_ssh_fields() {
    let f = ConnectionForm {
        ssh_port: "abc".into(),
        ..valid()
    };
    assert!(f.to_config().unwrap().ssh.is_none());
}

#[test]
fn ssh_key_path_selects_private_key_auth() {
    let f = ConnectionForm {
        ssh_key_path: "/k/id".into(),
        ..ssh_form()
    };
    let ssh = f.to_config().unwrap().ssh.unwrap();
    assert_eq!(
        ssh.auth,
        SshAuth::PrivateKey {
            path: "/k/id".into()
        }
    );
    assert_eq!(
        ssh_form().to_config().unwrap().ssh.unwrap().auth,
        SshAuth::Password
    );
}

#[test]
fn from_config_round_trips_without_secrets() {
    let f = ConnectionForm {
        auth: AuthMechanism::Plain,
        username: "bob".into(),
        password: "pw".into(),
        ssh_secret: "s".into(),
        clear_password: true,
        ssh_key_path: "/k".into(),
        ..ssh_form()
    };
    let cfg = f.to_config().unwrap();
    let back = ConnectionForm::from_config(&cfg);
    assert!(back.password.is_empty() && back.ssh_secret.is_empty());
    assert_eq!(back.password_edit(), SecretEdit::Unchanged);
    assert_eq!(back.ssh_secret_edit(), SecretEdit::Unchanged);
    assert_eq!(back.to_config().unwrap(), cfg);
}

#[test]
fn debug_redacts_secrets() {
    let f = ConnectionForm {
        password: "hunter2@:/%".into(),
        ssh_secret: "sshsecret".into(),
        ..valid()
    };
    let d = format!("{f:?} {f:#?}");
    assert!(!d.contains("hunter2") && !d.contains("sshsecret"));
}

#[test]
fn password_with_special_chars_round_trips_to_store_only() {
    let pw = "p@ss:w/rd%41 ü";
    let f = ConnectionForm {
        username: "bob".into(),
        password: pw.into(),
        ..valid()
    };
    let cfg = f.to_config().unwrap();
    let store = MemorySecretStore::new();
    save_connection(&cfg, &f, &store).unwrap();
    assert_eq!(
        store
            .get(&secret_key(cfg.id, SecretKind::Password))
            .unwrap()
            .as_deref(),
        Some(pw)
    );
    assert!(!serde_json::to_string(&cfg).unwrap().contains(pw));
}

#[test]
fn clear_password_deletes_stored_secret_and_is_idempotent() {
    let f = ConnectionForm {
        clear_password: true,
        ..valid()
    };
    let cfg = f.to_config().unwrap();
    let store = MemorySecretStore::new();
    let key = secret_key(cfg.id, SecretKind::Password);
    store.set(&key, "old").unwrap();
    save_connection(&cfg, &f, &store).unwrap();
    assert_eq!(store.get(&key).unwrap(), None);
    save_connection(&cfg, &f, &store).unwrap();
    assert_eq!(store.get(&key).unwrap(), None);
}

#[test]
fn editing_without_touching_secrets_keeps_both() {
    let orig = ConnectionForm {
        password: "pw".into(),
        ssh_secret: "sp".into(),
        ..ssh_form()
    };
    let cfg = orig.to_config().unwrap();
    let store = MemorySecretStore::new();
    save_connection(&cfg, &orig, &store).unwrap();

    let mut edited = ConnectionForm::from_config(&cfg);
    edited.name = "renamed".into();
    assert_eq!(edited.password_edit(), SecretEdit::Unchanged);
    let cfg2 = edited.to_config().unwrap();
    save_connection(&cfg2, &edited, &store).unwrap();
    let get = |k| store.get(&secret_key(cfg.id, k)).unwrap();
    assert_eq!(get(SecretKind::Password).as_deref(), Some("pw"));
    assert_eq!(get(SecretKind::SshPassword).as_deref(), Some("sp"));
}

#[test]
fn set_overwrites_verbatim() {
    let store = MemorySecretStore::new();
    let f = ConnectionForm {
        password: "n@w:/%41 ü ".into(),
        ..valid()
    };
    let cfg = f.to_config().unwrap();
    let key = secret_key(cfg.id, SecretKind::Password);
    store.set(&key, "old").unwrap();
    save_connection(&cfg, &f, &store).unwrap();
    assert_eq!(store.get(&key).unwrap().as_deref(), Some("n@w:/%41 ü "));
}

#[test]
fn new_connection_without_password_stores_nothing() {
    let f = valid();
    let cfg = f.to_config().unwrap();
    let store = MemorySecretStore::new();
    save_connection(&cfg, &f, &store).unwrap();
    assert_eq!(
        stored_secret_flags(cfg.id, &store).unwrap(),
        StoredSecrets::default()
    );
}

#[test]
fn stored_flags_report_presence_only() {
    let store = MemorySecretStore::new();
    let id = Uuid::new_v4();
    store
        .set(&secret_key(id, SecretKind::Password), "x")
        .unwrap();
    store
        .set(&secret_key(id, SecretKind::SshPassphrase), "y")
        .unwrap();
    assert_eq!(
        stored_secret_flags(id, &store).unwrap(),
        StoredSecrets {
            password: true,
            ssh_secret: true
        }
    );
}

#[test]
fn secret_edit_debug_is_redacted() {
    assert!(!format!("{:?}", SecretEdit::Set("hunter2".into())).contains("hunter2"));
}

#[test]
fn auth_source_and_invalid_certs_round_trip() {
    let f = ConnectionForm {
        auth_source: "admin".into(),
        allow_invalid_certs: true,
        ..valid()
    };
    let cfg = f.to_config().unwrap();
    assert_eq!(cfg.auth_source.as_deref(), Some("admin"));
    assert!(cfg.tls.allow_invalid_certs);
    let back = ConnectionForm::from_config(&cfg);
    assert_eq!(back.auth_source, "admin");
    assert!(back.allow_invalid_certs);
    assert_eq!(back.to_config().unwrap(), cfg);
}

#[test]
fn name_with_dots_and_spaces_is_accepted() {
    let f = ConnectionForm {
        name: "prod.eu west 1 ".into(),
        ..valid()
    };
    assert_eq!(f.to_config().unwrap().name, "prod.eu west 1");
}

#[test]
fn ssh_secret_kind_follows_auth_type() {
    let store = MemorySecretStore::new();
    let f = ConnectionForm {
        ssh_secret: "sp@:".into(),
        ..ssh_form()
    };
    let cfg = f.to_config().unwrap();
    save_connection(&cfg, &f, &store).unwrap();
    let pw_key = secret_key(cfg.id, SecretKind::SshPassword);
    let pp_key = secret_key(cfg.id, SecretKind::SshPassphrase);
    assert_eq!(store.get(&pw_key).unwrap().as_deref(), Some("sp@:"));
    assert_eq!(store.get(&pp_key).unwrap(), None);

    let f = ConnectionForm {
        ssh_key_path: "/k".into(),
        ..f
    };
    let cfg = f.to_config().unwrap();
    save_connection(&cfg, &f, &store).unwrap();
    assert_eq!(store.get(&pp_key).unwrap().as_deref(), Some("sp@:"));
    assert_eq!(store.get(&pw_key).unwrap(), None, "stale kind removed");
}

#[test]
fn ssh_disabled_clears_ssh_secrets() {
    let store = MemorySecretStore::new();
    let f = valid();
    let cfg = f.to_config().unwrap();
    let k = secret_key(cfg.id, SecretKind::SshPassword);
    store.set(&k, "x").unwrap();
    save_connection(&cfg, &f, &store).unwrap();
    assert_eq!(store.get(&k).unwrap(), None);
}

#[test]
fn auth_options_follow_gssapi_availability() {
    let opts = auth_options();
    assert_eq!(
        opts.contains(&AuthMechanism::Gssapi),
        curlybrace_core::GSSAPI_AVAILABLE
    );
    assert!(opts.contains(&AuthMechanism::None) && opts.contains(&AuthMechanism::Plain));
}

#[test]
fn every_auth_label_key_exists_in_locale() {
    let i18n = I18n::english();
    for a in [
        AuthMechanism::None,
        AuthMechanism::ScramSha1,
        AuthMechanism::ScramSha256,
        AuthMechanism::X509,
        AuthMechanism::Gssapi,
        AuthMechanism::Plain,
    ] {
        assert!(!i18n.t(auth_label_key(a)).contains('⟦'), "{a:?}");
    }
}

#[test]
fn form_round_trips_folder_and_timestamps() {
    let mut c = ConnectionConfig::new("a", "mongodb://h").with_save_stamp(10);
    c.folder = Some("work".into());
    c.last_connected = Some(99);
    let back = ConnectionForm::from_config(&c).to_config().unwrap();
    assert_eq!(back.folder.as_deref(), Some("work"));
    assert_eq!(back.created_at, Some(10));
    assert_eq!(back.last_connected, Some(99));
}

#[test]
fn blank_folder_text_becomes_none() {
    let mut form = ConnectionForm::from_config(&ConnectionConfig::new("a", "mongodb://h"));
    form.folder = "   ".into();
    assert_eq!(form.to_config().unwrap().folder, None);
}

#[test]
fn default_form_is_uncoloured_non_production() {
    let f = ConnectionForm::default();
    assert_eq!((f.color, f.production), (None, false));
    let cfg = valid().to_config().unwrap();
    assert_eq!((cfg.color, cfg.production), (None, false));
}

#[test]
fn form_round_trips_color_and_production() {
    for color in ConnectionColor::ALL {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.color = Some(SavedColor::Known(color));
        c.production = true;
        let form = ConnectionForm::from_config(&c);
        assert_eq!(form.color, Some(SavedColor::Known(color)));
        assert!(form.production);
        assert_eq!(form.to_config().unwrap(), c);
    }
}

#[test]
fn new_form_defaults_read_only_false() {
    assert!(!ConnectionForm::default().read_only);
    assert!(!valid().to_config().unwrap().read_only);
}

#[test]
fn form_round_trips_read_only() {
    let mut c = ConnectionConfig::new("a", "mongodb://h");
    c.read_only = true;
    let form = ConnectionForm::from_config(&c);
    assert!(form.read_only);
    assert_eq!(form.to_config().unwrap(), c);
}

#[test]
fn clearing_the_colour_clears_an_existing_colour() {
    let mut c = ConnectionConfig::new("a", "mongodb://h");
    c.color = Some(SavedColor::Known(ConnectionColor::Blue));
    let mut form = ConnectionForm::from_config(&c);
    form.color = None;
    assert_eq!(form.to_config().unwrap().color, None);
}

fn unrecognised() -> ConnectionConfig {
    let mut c = ConnectionConfig::new("a", "mongodb://h");
    c.color = Some(SavedColor::Unrecognised("future-shade".into()));
    c
}

#[test]
fn an_untouched_form_keeps_an_unrecognised_colour_on_save() {
    let c = unrecognised();
    let form = ConnectionForm::from_config(&c);
    assert_eq!(form.to_config().unwrap(), c);
}

#[test]
fn picking_a_preset_replaces_an_unrecognised_colour() {
    let mut form = ConnectionForm::from_config(&unrecognised());
    form.color = Some(SavedColor::Known(ConnectionColor::Green));
    assert_eq!(
        form.to_config().unwrap().color,
        Some(SavedColor::Known(ConnectionColor::Green))
    );
}

#[test]
fn clearing_the_colour_removes_an_unrecognised_one() {
    let mut form = ConnectionForm::from_config(&unrecognised());
    form.color = None;
    assert_eq!(form.to_config().unwrap().color, None);
}
