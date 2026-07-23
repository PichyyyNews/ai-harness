import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { RetrievalTraceEntry, WebSource } from "../types";

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

export interface InteractionOption {
  id: string;
  label: string;
}

interface StreamChatOptions {
  messages: ChatMessage[];
  sessionId?: string;
  interactionId?: string;
  interactionOptionId?: string;
  signal: AbortSignal;
  onDelta: (delta: string) => void;
  onTrim: (suffix: string) => void;
  onStatus: (status: string) => void;
  onRetrievalTrace: (entry: RetrievalTraceEntry) => void;
}

interface EngineTokenEvent { token: string; }
interface EngineTrimEvent { suffix: string; }
interface EngineStatusEvent { status: string; }

export interface GenerationResult {
  content: string;
  finishReason: "stop" | "length" | "repetition_detected" | "cancelled" | string;
  sources: WebSource[];
  retrievalTrace: RetrievalTraceEntry[];
}

export async function streamLocalChat({ messages, sessionId, interactionId, interactionOptionId, signal, onDelta, onTrim, onStatus, onRetrievalTrace }: StreamChatOptions): Promise<GenerationResult | undefined> {
  const [unlistenToken, unlistenTrim, unlistenStatus, unlistenTrace] = await Promise.all([
    listen<EngineTokenEvent>("engine-token", (event) => onDelta(event.payload.token)),
    listen<EngineTrimEvent>("engine-trim", (event) => onTrim(event.payload.suffix)),
    listen<EngineStatusEvent>("engine-status", (event) => onStatus(event.payload.status)),
    listen<RetrievalTraceEntry>("retrieval-trace", (event) => onRetrievalTrace(event.payload)),
  ]);
  const stop = () => { void invoke("stop_generation"); };
  signal.addEventListener("abort", stop, { once: true });
  try {
    if (signal.aborted) {
      stop();
      return undefined;
    }
    return await invoke<GenerationResult>("generate_chat", { request: { messages, maxTokens: 1536, temperature: 0.75, sessionId, interactionId, interactionOptionId } });
  } finally {
    signal.removeEventListener("abort", stop);
    unlistenToken();
    unlistenTrim();
    unlistenStatus();
    unlistenTrace();
  }
}
