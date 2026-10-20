use super::*;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn xdg_paths_and_account_names() {
    if std::env::var_os("SEN_TEST_LOCATIONS_CHILD").is_some() {
        let root = PathBuf::from(std::env::var_os("SEN_TEST_LOCATIONS_CHILD").unwrap());
        let locations = Locations::from_environment().unwrap();
        assert_eq!(locations.data, root.join("data/sen"));
        assert_eq!(locations.session, root.join("data/sen/auth/session.json"));
        assert_eq!(locations.cache, root.join("cache/sen"));
        assert_eq!(locations.config_home, root.join("config"));
        assert_eq!(
            account_directory(&locations.data, "U123").unwrap(),
            root.join("data/sen/U123")
        );
        assert_ne!(
            account_directory(&locations.data, "U123").unwrap(),
            account_directory(&locations.data, "U456").unwrap()
        );
        for invalid in [
            "", ".", "..", "../U123", "U/123", "U\\123", "/U123", "U\n123",
        ] {
            assert!(
                account_directory(&locations.data, invalid).is_err(),
                "{invalid:?}"
            );
        }
        return;
    }
    let root = crate::test_support::TempDir::new("locations");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "locations::tests::xdg_paths_and_account_names"])
        .env("SEN_TEST_LOCATIONS_CHILD", &root.0)
        .env("HOME", &root.0)
        .env("XDG_DATA_HOME", root.0.join("data"))
        .env("XDG_CACHE_HOME", root.0.join("cache"))
        .env("XDG_CONFIG_HOME", root.0.join("config"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn first_start_copies_private_auth_and_keeps_both_existing_files() {
    let root = crate::test_support::TempDir::new("first-start");
    let old = root.0.join("line-desktop");
    fs::create_dir_all(old.join("accounts")).unwrap();
    let session = b"synthetic sealed session bytes";
    fs::write(old.join("session.json"), session).unwrap();
    fs::write(old.join("accounts/last-mid"), b"U123").unwrap();
    let locations = Locations {
        config_home: root.0.join("config"),
        data: root.0.join("sen"),
        session: root.0.join("sen/auth/session.json"),
        cache: root.0.join("cache/sen"),
    };
    locations.copy_first_start().unwrap();
    assert_eq!(fs::read(&locations.session).unwrap(), session);
    assert_eq!(
        fs::metadata(&locations.session)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read(locations.data.join("auth/last-account")).unwrap(),
        b"U123"
    );
    fs::write(&locations.session, b"new session").unwrap();
    fs::write(locations.data.join("auth/last-account"), b"U456").unwrap();
    locations.copy_first_start().unwrap();
    assert_eq!(fs::read(&locations.session).unwrap(), b"new session");
    assert_eq!(
        fs::read(locations.data.join("auth/last-account")).unwrap(),
        b"U456"
    );
    assert_eq!(fs::read(old.join("session.json")).unwrap(), session);
    assert_eq!(fs::read(old.join("accounts/last-mid")).unwrap(), b"U123");
}
