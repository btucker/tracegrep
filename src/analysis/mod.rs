pub mod codepaths;

use std::path::Path;

/// Returns true if the relative file path looks like a test file.
/// Checks directory patterns (tests/, test/, __tests__/) and
/// filename patterns (_test, .test, .spec, _spec suffixes; test_ prefix).
pub fn is_test_file(relative_path: &str) -> bool {
    let path_with_slash = if relative_path.starts_with('/') {
        relative_path.to_string()
    } else {
        format!("/{relative_path}")
    };
    if path_with_slash.contains("/tests/")
        || path_with_slash.contains("/test/")
        || path_with_slash.contains("/__tests__/")
    {
        return true;
    }

    let file_name = Path::new(relative_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");

    let stem = if let Some(dot_pos) = file_name.rfind('.') {
        &file_name[..dot_pos]
    } else {
        file_name
    };

    if stem.is_empty() {
        return false;
    }

    if stem.ends_with("_test")
        || stem.ends_with(".test")
        || stem.ends_with(".spec")
        || stem.ends_with("_spec")
    {
        return true;
    }

    stem.starts_with("test_")
}

#[cfg(test)]
mod tests {
    use super::is_test_file;

    #[test]
    fn detects_test_paths() {
        assert!(is_test_file("tests/foo.rs"));
        assert!(is_test_file("src/test/bar.py"));
        assert!(is_test_file("src/__tests__/baz.ts"));
        assert!(is_test_file("src/foo_test.rs"));
        assert!(is_test_file("src/foo.spec.ts"));
        assert!(is_test_file("tests/test_foo.py"));
    }

    #[test]
    fn ignores_non_test_paths() {
        assert!(!is_test_file("src/main.rs"));
        assert!(!is_test_file("src/latest.rs"));
        assert!(!is_test_file("Cargo.toml"));
    }
}
