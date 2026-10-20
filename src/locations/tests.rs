use super::*;
use std::fs;

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
fn logout_deletion_stays_inside_the_account_roots() {
    let temp = crate::test_support::TempDir::new("logout-paths");
    for root in [temp.0.join("data/sen"), temp.0.join("cache/sen")] {
        for mid in ["self", "other"] {
            temp.write(&root.join(mid).join("history"), b"other");
        }
        delete_account(&root, "self").unwrap();
        assert!(!root.join("self").exists());
        assert_eq!(fs::read(root.join("other/history")).unwrap(), b"other");
        assert!(delete_account(&root, "../other").is_err());
        std::os::unix::fs::symlink(root.join("other"), root.join("self")).unwrap();
        // Even a sibling symlink must not delete the target's data.
        assert!(delete_account(&root, "self").is_err());
        assert!(root.join("other/history").exists());
    }

    for name in ["auth", "images", "fonts", "clipboard-attachments"] {
        temp.write(&temp.0.join(name).join("keep"), b"private");
        assert!(delete_account(&temp.0, name).is_err(), "{name}");
        assert!(temp.0.join(name).join("keep").exists());
    }
}
