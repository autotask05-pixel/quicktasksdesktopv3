use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Mutex;
use tracing::{field::Field, Event, Subscriber};
use tracing_subscriber::field::Visit;
use tracing_subscriber::layer::{Context, Layer};

const MAX_LOG_ENTRIES: usize = 400;

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub id: u64,
    pub line: String,
}

#[derive(Default)]
struct LogBuffer {
    next_id: u64,
    entries: VecDeque<(u64, String)>,
}

static LOG_BUFFER: Lazy<Mutex<LogBuffer>> = Lazy::new(|| Mutex::new(LogBuffer::default()));

pub fn push_log(line: String) {
    let mut buffer = LOG_BUFFER.lock().expect("log buffer poisoned");
    let entry = (buffer.next_id, line);
    buffer.next_id += 1;
    buffer.entries.push_back(entry);
    while buffer.entries.len() > MAX_LOG_ENTRIES {
        buffer.entries.pop_front();
    }
}

pub fn read_logs(since: Option<u64>) -> (u64, Vec<LogEntry>) {
    let buffer = LOG_BUFFER.lock().expect("log buffer poisoned");
    let next_cursor = buffer.next_id;
    let entries = buffer
        .entries
        .iter()
        .filter(|(id, _)| since.map(|cursor| *id >= cursor).unwrap_or(true))
        .map(|(id, line)| LogEntry {
            id: *id,
            line: line.clone(),
        })
        .collect();
    (next_cursor, entries)
}

pub fn layer() -> UiLogLayer {
    UiLogLayer
}

pub struct UiLogLayer;

impl<S> Layer<S> for UiLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);

        let line = if visitor.fields.is_empty() {
            format!(
                "[{} {}] {}",
                metadata.level(),
                metadata.target(),
                metadata.name()
            )
        } else {
            format!(
                "[{} {}] {}",
                metadata.level(),
                metadata.target(),
                visitor.fields.join(" ")
            )
        };

        push_log(line);
    }
}

#[derive(Default)]
struct LogVisitor {
    fields: Vec<String>,
}

impl Visit for LogVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields.push(format!("{}={:?}", field.name(), value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.push(format!("{}={}", field.name(), value));
    }
}
