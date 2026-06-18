use crate::config::LoggingConfig;

/// Resolve the effective EnvFilter string from the priority chain:
/// 1. --log-level CLI flag (highest)
/// 2. -v / -vv CLI flags
/// 3. QUMA_LOG_LEVEL env var (already applied to config by apply_env_overrides)
/// 4. RUST_LOG env var
/// 5. config.logging.level
/// 6. hardcoded default: "info,quartermaster=debug"
pub fn resolve_log_filter(config: &LoggingConfig, verbose: u8, log_level: Option<&str>) -> String {
    // Priority 1: --log-level flag (global, overrides everything)
    if let Some(level) = log_level {
        return level.to_string();
    }

    // Determine base filter from priority chain
    let base = if std::env::var("QUMA_LOG_LEVEL").is_ok() {
        // Priority 3: QUMA_LOG_LEVEL (already merged into config.level via apply_env_overrides)
        // Takes precedence over RUST_LOG
        format!("{},quartermaster=debug", config.level)
    } else if let Ok(rust_log) = std::env::var("RUST_LOG") {
        // Priority 4: RUST_LOG (full filter syntax, legacy escape hatch)
        rust_log
    } else if config.level != "info" {
        // Priority 5: config file level
        format!("{},quartermaster=debug", config.level)
    } else {
        // Priority 6: hardcoded default
        "info,quartermaster=debug".to_string()
    };

    // Priority 2: -v / -vv (crate-scoped override)
    match verbose {
        0 => base,
        1 => {
            let without_qm = strip_crate_directive(&base, "quartermaster");
            format!("{without_qm},quartermaster=debug")
        }
        _ => {
            let without_qm = strip_crate_directive(&base, "quartermaster");
            format!("{without_qm},quartermaster=trace")
        }
    }
}

fn strip_crate_directive(filter: &str, crate_name: &str) -> String {
    filter
        .split(',')
        .filter(|part| {
            !part
                .split('=')
                .next()
                .is_some_and(|key| key.trim() == crate_name)
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "info,quartermaster=debug");
            },
        );
    }

    #[test]
    fn config_level_overrides_default() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let mut config = LoggingConfig::default();
                config.level = "warn".to_string();
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "warn,quartermaster=debug");
            },
        );
    }

    #[test]
    fn verbose_flag_sets_crate_debug() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 1, None);
                assert_eq!(result, "info,quartermaster=debug");
            },
        );
    }

    #[test]
    fn double_verbose_sets_crate_trace() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 2, None);
                assert_eq!(result, "info,quartermaster=trace");
            },
        );
    }

    #[test]
    fn log_level_flag_overrides_everything() {
        temp_env::with_vars(
            [("RUST_LOG", None::<&str>), ("QUMA_LOG_LEVEL", None::<&str>)],
            || {
                let mut config = LoggingConfig::default();
                config.level = "warn".to_string();
                let result = resolve_log_filter(&config, 2, Some("error"));
                assert_eq!(result, "error");
            },
        );
    }

    #[test]
    fn rust_log_env_overrides_config() {
        temp_env::with_vars(
            [
                ("RUST_LOG", Some("debug,hyper=warn")),
                ("QUMA_LOG_LEVEL", None),
            ],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "debug,hyper=warn");
            },
        );
    }

    #[test]
    fn verbose_overrides_rust_log() {
        temp_env::with_vars(
            [("RUST_LOG", Some("warn")), ("QUMA_LOG_LEVEL", None)],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 1, None);
                assert_eq!(result, "warn,quartermaster=debug");
            },
        );
    }

    #[test]
    fn log_level_flag_overrides_rust_log() {
        temp_env::with_vars(
            [
                ("RUST_LOG", Some("debug,hyper=warn")),
                ("QUMA_LOG_LEVEL", None),
            ],
            || {
                let config = LoggingConfig::default();
                let result = resolve_log_filter(&config, 0, Some("trace"));
                assert_eq!(result, "trace");
            },
        );
    }

    #[test]
    fn quma_log_level_overrides_rust_log() {
        temp_env::with_vars(
            [
                ("RUST_LOG", Some("debug,hyper=warn")),
                ("QUMA_LOG_LEVEL", Some("error")),
            ],
            || {
                let mut config = LoggingConfig::default();
                config.level = "error".to_string(); // simulates apply_env_overrides
                let result = resolve_log_filter(&config, 0, None);
                assert_eq!(result, "error,quartermaster=debug");
            },
        );
    }
}
