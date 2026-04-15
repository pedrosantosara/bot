use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::models::LogEntry;
use crate::web::state::AppState;

/// A tracing layer that captures log events and pushes them to the AppState log buffer.
pub struct WebLogLayer {
    state: AppState,
    rt: tokio::runtime::Handle,
}

impl WebLogLayer {
    pub fn new(state: AppState, rt: tokio::runtime::Handle) -> Self {
        Self { state, rt }
    }
}

struct MessageVisitor {
    message: String,
    fields: Vec<String>,
}

impl MessageVisitor {
    fn new() -> Self {
        Self { message: String::new(), fields: Vec::new() }
    }

    fn result(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else {
            format!("{} {}", self.message, self.fields.join(" "))
        }
    }
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}

impl<S: Subscriber> Layer<S> for WebLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = meta.level().to_string();

        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);
        let message = visitor.result();

        // Skip empty messages and WebSocket noise
        if message.is_empty() {
            return;
        }

        let entry = LogEntry {
            timestamp: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            level,
            message,
        };

        let state = self.state.clone();
        self.rt.spawn(async move {
            state.push_log(entry).await;
        });
    }
}
