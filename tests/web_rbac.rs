use spt_quartermaster::db::rbac::Permission;
use std::collections::HashSet;

/// Validates that all permission strings used in templates via `user.can("...")`
/// correspond to valid Permission enum variants. Catches typos at test time.
#[test]
fn template_permission_strings_are_valid() {
    let valid_slugs: HashSet<&str> = Permission::ALL.iter().map(|p| p.as_str()).collect();

    let template_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut errors = Vec::new();

    for entry in walkdir::WalkDir::new(&template_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "html"))
    {
        let content = std::fs::read_to_string(entry.path()).unwrap();
        // Match user.can("...") patterns
        for cap in regex::Regex::new(r#"user\.can\("([^"]+)"\)"#)
            .unwrap()
            .captures_iter(&content)
        {
            let slug = cap.get(1).unwrap().as_str();
            if !valid_slugs.contains(slug) {
                errors.push(format!(
                    "{}:  unknown permission slug \"{}\"",
                    entry.path().strip_prefix(&template_dir).unwrap().display(),
                    slug
                ));
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Template permission string typos found:\n{}",
        errors.join("\n")
    );
}
