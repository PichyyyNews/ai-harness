import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

interface StreamChatOptions {
  messages: ChatMessage[];
  sessionId?: string;
  signal: AbortSignal;
  onDelta: (delta: string) => void;
  onTrim: (suffix: string) => void;
  onStatus: (status: string) => void;
}

interface EngineTokenEvent { token: string; }
interface EngineTrimEvent { suffix: string; }
interface EngineStatusEvent { status: string; }

export interface GenerationResult {
  content: string;
  finishReason: "stop" | "length" | "repetition_detected" | "cancelled" | string;
}

export async function streamLocalChat({ messages, sessionId, signal, onDelta, onTrim, onStatus }: StreamChatOptions): Promise<GenerationResult | undefined> {
  const [unlistenToken, unlistenTrim, unlistenStatus] = await Promise.all([
    listen<EngineTokenEvent>("engine-token", (event) => onDelta(event.payload.token)),
    listen<EngineTrimEvent>("engine-trim", (event) => onTrim(event.payload.suffix)),
    listen<EngineStatusEvent>("engine-status", (event) => onStatus(event.payload.status)),
  ]);
  const stop = () => { void invoke("stop_generation"); };
  signal.addEventListener("abort", stop, { once: true });
  try {
    if (signal.aborted) {
      stop();
      return undefined;
    }
    return await invoke<GenerationResult>("generate_chat", { request: { messages, maxTokens: 1536, temperature: 0.75, sessionId } });
  } finally {
    signal.removeEventListener("abort", stop);
    unlistenToken();
    unlistenTrim();
    unlistenStatus();
  }
}
