use crate::language_classifier::MessageClassification;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Sender},
    Arc,
};
use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct MemoryUpdateJob {
    pub session_id: String,
    pub user_message: String,
    pub assistant_response: String,
    pub turn_index: usize,
    pub classification: Option<MessageClassification>,
}

#[derive(Clone)]
pub struct MemoryAgentHandle {
    sender: Sender<MemoryAgentJob>,
}

#[derive(Debug, Clone)]
enum MemoryAgentJob {
    Turn(MemoryUpdateJob),
    SessionEnd { session_id: String },
}

impl MemoryAgentHandle {
    pub fn start(
        app: AppHandle,
        endpoint: String,
        embedding_endpoint: Option<String>,
        generation_active: Arc<AtomicBool>,
        shares_main_endpoint: bool,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<MemoryAgentJob>();
        std::thread::Builder::new()
            .name("memory-agent".to_string())
            .spawn(move || {
                while let Ok(first) = receiver.recv() {
                    let mut batch = vec![first];
                    while let Ok(job) = receiver.try_recv() {
                        batch.push(job);
                    }

                    // Tier A always runs for the complete queued batch first.
                    // This bounds constraint lag even when Tier B/C are busy.
                    for job in batch.iter().filter_map(|job| match job {
                        MemoryAgentJob::Turn(job) => Some(job),
                        MemoryAgentJob::SessionEnd { .. } => None,
                    }) {
                        wait_for_chat_lane(&generation_active, shares_main_endpoint);
                        if let Err(error) = super::worker::run_constraint_scan(
                            &app,
                            &endpoint,
                            &job.session_id,
                            &job.user_message,
                            job.classification.as_ref(),
                        ) {
                            super::observability::log_error(
                                &app,
                                &job.session_id,
                                "constraint_scan",
                                &error,
                            );
                            eprintln!("[memory-agent] constraint scan skipped: {error}");
                        }
                    }

                    for job in batch.iter().filter_map(|job| match job {
                        MemoryAgentJob::Turn(job) => Some(job),
                        MemoryAgentJob::SessionEnd { .. } => None,
                    }) {
                        if should_update_mid_term(job.turn_index) {
                            wait_for_chat_lane(&generation_active, shares_main_endpoint);
                            if let Err(error) = super::worker::run_mid_term_scan(
                                &app,
                                &endpoint,
                                &job.session_id,
                                &job.user_message,
                                &job.assistant_response,
                            ) {
                                super::observability::log_error(
                                    &app,
                                    &job.session_id,
                                    "mid_term_scan",
                                    &error,
                                );
                                eprintln!("[memory-agent] mid-term scan skipped: {error}");
                            }
                        }
                    }

                    for job in batch.iter().filter_map(|job| match job {
                        MemoryAgentJob::Turn(job) => Some(job),
                        MemoryAgentJob::SessionEnd { .. } => None,
                    }) {
                        if should_scan_long_term(job.turn_index) {
                            wait_for_chat_lane(&generation_active, shares_main_endpoint);
                            if let Err(error) = super::worker::run_session_end_extraction(
                                &app,
                                &endpoint,
                                embedding_endpoint.as_deref(),
                                &job.session_id,
                            ) {
                                super::observability::log_error(
                                    &app,
                                    &job.session_id,
                                    "long_term_scan",
                                    &error,
                                );
                                eprintln!("[memory-agent] long-term scan skipped: {error}");
                            }
                        }
                    }

                    // Session-end extraction shares the same FIFO lane as the
                    // after-turn scans. This guarantees the final turn is in
                    // SQLite before durable facts and the summary are built,
                    // and prevents concurrent inference against one endpoint.
                    for session_id in batch.iter().filter_map(|job| match job {
                        MemoryAgentJob::SessionEnd { session_id } => Some(session_id),
                        MemoryAgentJob::Turn(_) => None,
                    }) {
                        wait_for_chat_lane(&generation_active, shares_main_endpoint);
                        if let Err(error) = super::worker::run_session_end_extraction(
                            &app,
                            &endpoint,
                            embedding_endpoint.as_deref(),
                            session_id,
                        ) {
                            super::observability::log_error(
                                &app,
                                session_id,
                                "session_end_scan",
                                &error,
                            );
                            eprintln!("[memory-agent] session-end scan skipped: {error}");
                        }
                    }
                }
            })
            .expect("memory agent thread should start");
        Self { sender }
    }

    pub fn enqueue(&self, job: MemoryUpdateJob) -> Result<(), String> {
        self.sender
            .send(MemoryAgentJob::Turn(job))
            .map_err(|_| "Memory agent queue is no longer available.".to_string())
    }

    pub fn enqueue_session_end(&self, session_id: String) -> Result<(), String> {
        self.sender
            .send(MemoryAgentJob::SessionEnd { session_id })
            .map_err(|_| "Memory agent queue is no longer available.".to_string())
    }
}

fn wait_for_chat_lane(generation_active: &AtomicBool, shares_main_endpoint: bool) {
    if !shares_main_endpoint {
        return;
    }
    while generation_active.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

fn should_update_mid_term(turn_index: usize) -> bool {
    turn_index == 1 || turn_index % 2 == 0
}

fn should_scan_long_term(turn_index: usize) -> bool {
    turn_index > 0 && turn_index % 5 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_cadence_starts_on_turn_one_and_bounds_expensive_work() {
        assert!(should_update_mid_term(1));
        assert!(!should_update_mid_term(3));
        assert!(should_update_mid_term(4));
        assert!(!should_scan_long_term(4));
        assert!(should_scan_long_term(5));
    }
}
