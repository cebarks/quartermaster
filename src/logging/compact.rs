use std::fmt;

use chrono::Local;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{self, FormatEvent, FormatFields};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::registry::LookupSpan;

pub struct CompactFormatter {
    pub use_ansi: bool,
}

impl<S, N> FormatEvent<S, N> for CompactFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: format::Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let level = metadata.level();
        let target = metadata.target();
        let now = Local::now();

        // Timestamp
        write!(writer, "{} ", now.format("%Y-%m-%d %H:%M:%S"))?;

        // Level with optional color
        if self.use_ansi {
            match *level {
                tracing::Level::ERROR => write!(writer, "\x1b[31mERROR\x1b[0m ")?,
                tracing::Level::WARN => write!(writer, "\x1b[33mWARN \x1b[0m ")?,
                tracing::Level::INFO => write!(writer, "\x1b[32mINFO \x1b[0m ")?,
                tracing::Level::DEBUG => write!(writer, "\x1b[90mDEBUG\x1b[0m ")?,
                tracing::Level::TRACE => write!(writer, "\x1b[90mTRACE\x1b[0m ")?,
            }
        } else {
            write!(writer, "{:<5} ", level)?;
        }

        // Extract fields — check if this is an HTTP request span from tracing-actix-web
        let mut visitor = CompactVisitor::new();
        event.record(&mut visitor);

        if is_http_request(target, &visitor.fields) {
            // HTTP request compact format: METHOD /path STATUS duration
            format_http_request(&mut writer, &visitor)?;
        } else {
            // Regular event: message key=value key=value
            write!(writer, "{}", visitor.message)?;
            for (key, value) in &visitor.fields {
                write!(writer, " {key}={value}")?;
            }
        }

        writeln!(writer)
    }
}

fn is_http_request(target: &str, fields: &[(String, String)]) -> bool {
    target.starts_with("tracing_actix_web")
        || target.starts_with("actix_web")
        || fields
            .iter()
            .any(|(k, _)| k == "http.method" || k == "http.status_code")
}

fn format_http_request(writer: &mut format::Writer<'_>, visitor: &CompactVisitor) -> fmt::Result {
    let method = visitor
        .get_field("http.method")
        .or_else(|| visitor.get_field("method"))
        .unwrap_or("?");
    let path = visitor
        .get_field("http.target")
        .or_else(|| visitor.get_field("http.route"))
        .or_else(|| visitor.get_field("path"))
        .unwrap_or("?");
    let status = visitor
        .get_field("http.status_code")
        .or_else(|| visitor.get_field("status_code"))
        .unwrap_or("?");
    let latency = visitor
        .get_field("http.latency")
        .or_else(|| visitor.get_field("latency"))
        .or_else(|| visitor.get_field("elapsed_milliseconds"));

    write!(writer, "{method} {path} {status}")?;
    if let Some(lat) = latency {
        write!(writer, " {lat}")?;
    }
    Ok(())
}

struct CompactVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl CompactVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
            fields: Vec::new(),
        }
    }

    fn get_field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

impl tracing::field::Visit for CompactVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if value.contains(' ') {
            self.fields
                .push((field.name().to_string(), format!("\"{value}\"")));
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .push((field.name().to_string(), value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tracing_subscriber::prelude::*;

    #[test]
    fn compact_format_regular_event() {
        let buf = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let buf_clone = Arc::clone(&buf);
        let make_writer =
            move || -> Box<dyn std::io::Write> { Box::new(TestWriter(Arc::clone(&buf_clone))) };

        let formatter = CompactFormatter { use_ansi: false };
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .event_format(formatter)
                .with_writer(make_writer)
                .with_ansi(false),
        );
        let _guard = tracing::subscriber::set_default(subscriber);

        tracing::info!(mod_id = 482, mod_name = "SAIN", "mod installed");

        let output = String::from_utf8(buf.lock().clone()).unwrap();
        // Should contain date, time, level, message, and fields
        assert!(output.contains("INFO"));
        assert!(output.contains("mod installed"));
        assert!(output.contains("mod_id=482"));
        assert!(output.contains("mod_name=SAIN")); // no quotes for single-word strings
                                                   // Should NOT contain target
        assert!(!output.contains("quartermaster"));
    }

    struct TestWriter(Arc<parking_lot::Mutex<Vec<u8>>>);
    impl std::io::Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
