use std::sync::Once;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

pub fn init_logging() {
    INIT.call_once(|| {
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,ai_harness_lib=debug"));

        let log_dir = std::env::temp_dir().join("aphelion_logs");
        let file_appender = tracing_appender::rolling::daily(&log_dir, "aphelion.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        // Retain guard in memory for app lifespan
        Box::leak(Box::new(_guard));

        let file_layer = fmt::layer()
            .with_ansi(false)
            .with_writer(non_blocking);

        let console_layer = fmt::layer()
            .with_ansi(true);

        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(console_layer)
            .with(file_layer)
            .try_init();

        tracing::info!("Structured logging initialized successfully at {:?}", log_dir);
    });
}
