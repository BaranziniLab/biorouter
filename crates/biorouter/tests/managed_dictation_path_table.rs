use biorouter::config::paths::Paths;
use std::path::PathBuf;

const TYPESCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../ui/desktop/src/utils/managedDictationPolicy.ts"
));

#[test]
fn laptop_path_table_matches_managed_policy_path() {
    assert!(TYPESCRIPT.contains("BIOROUTER_PATH_ROOT"));
    assert!(TYPESCRIPT.contains("path.join(root, 'managed', 'managed-policy.yaml')"));
    let override_root = std::env::var("BIOROUTER_PATH_ROOT")
        .ok()
        .filter(|root| !root.trim().is_empty());
    if let Some(root) = override_root {
        assert_eq!(
            Paths::managed_policy_path(),
            Some(PathBuf::from(root).join("managed/managed-policy.yaml"))
        );
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let expected = "/Library/Application Support/Biorouter/managed-policy.yaml";
        assert!(TYPESCRIPT.contains(expected));
        assert_eq!(Paths::managed_policy_path(), Some(PathBuf::from(expected)));
    }
    #[cfg(target_os = "linux")]
    {
        let expected = "/etc/biorouter/managed-policy.yaml";
        assert!(TYPESCRIPT.contains(expected));
        assert_eq!(Paths::managed_policy_path(), Some(PathBuf::from(expected)));
    }
    #[cfg(target_os = "windows")]
    {
        assert!(
            TYPESCRIPT.contains("path.join(env.ProgramData, 'Biorouter', 'managed-policy.yaml')")
        );
        let expected = std::env::var("ProgramData").ok().map(|root| {
            PathBuf::from(root)
                .join("Biorouter")
                .join("managed-policy.yaml")
        });
        assert_eq!(Paths::managed_policy_path(), expected);
    }
}
