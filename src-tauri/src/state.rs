use crate::engine::{
    context_manager::ConversationMemory, embedding_runtime::EmbeddingRuntime,
    memory::agent::MemoryAgentHandle, time_manager::TimeAuthority, Engine,
};
use std::sync::{atomic::AtomicBool, Arc, Mutex};

pub struct EngineState {
    pub engine: Mutex<Option<Engine>>,
    pub conversation_memory: Mutex<ConversationMemory>,
    pub memory_agent: Mutex<Option<MemoryAgentHandle>>,
    pub embedding_runtime: Mutex<Option<EmbeddingRuntime>>,
    pub time_authority: Mutex<TimeAuthority>,
    pub cancel_generation: AtomicBool,
    pub generation_active: Arc<AtomicBool>,
    pub memory_injection_enabled: AtomicBool,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            engine: Mutex::new(None),
            conversation_memory: Mutex::new(ConversationMemory::default()),
            memory_agent: Mutex::new(None),
            embedding_runtime: Mutex::new(None),
            time_authority: Mutex::new(TimeAuthority::default()),
            cancel_generation: AtomicBool::new(false),
            generation_active: Arc::new(AtomicBool::new(false)),
            memory_injection_enabled: AtomicBool::new(true),
        }
    }
}
