use crate::engine::{context_manager::ConversationMemory, time_manager::TimeAuthority, Engine};
use std::sync::{atomic::AtomicBool, Mutex};

pub struct EngineState {
    pub engine: Mutex<Option<Engine>>,
    pub conversation_memory: Mutex<ConversationMemory>,
    pub time_authority: Mutex<TimeAuthority>,
    pub cancel_generation: AtomicBool,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            engine: Mutex::new(None),
            conversation_memory: Mutex::new(ConversationMemory::default()),
            time_authority: Mutex::new(TimeAuthority::default()),
            cancel_generation: AtomicBool::new(false),
        }
    }
}
