# 02. Agentic Loop & Execution Mechanics

## Overview

The **Agentic Loop** (`src-tauri/src/tools/agent_loop.rs`) is the central intelligence engine of Aphelion. It allows the local LLM to reason in multiple hops, invoke external tools (such as web search or workspace access), process tool outputs, auto-continue truncated text responses, and suspend execution to prompt the user natively via interactive UI widgets.

---

## Agent Loop Pipeline Architecture

```
                      ┌─────────────────────────┐
                      │  run_agentic_loop(...)  │
                      └────────────┬────────────┘
                                   │
                                   ▼
                       ┌───────────────────────┐
                       │  Iteration Loop (<=8) │
                       └───────────┬───────────┘
                                   │
                                   ▼
                   ┌───────────────────────────────┐
                   │  request_chat_completion()    │
                   │  (max_tokens: 4096, temp: 0.7)│
                   └───────────────┬───────────────┘
                                   │
                                   ▼
                   ┌───────────────────────────────┐
                   │  parse_loop_step_response()   │
                   └───────────────┬───────────────┘
                                   │
                ┌──────────────────┴──────────────────┐
                ▼                                     ▼
   [ LoopStepResult::ToolCalls ]         [ LoopStepResult::FinalAnswer ]
                │                                     │
   ┌────────────┴────────────┐             ┌──────────┴──────────┐
   │ Check for User Choice UI│             │ Check Empty Answer  │
   │ (`ask_user_clarification`)            │ (`force_final_ans`) │
   └────────────┬────────────┘             └──────────┬──────────┘
                │                                     │
       Is Choice Call?                     ┌──────────┴──────────┐
      ┌─────────┴─────────┐                │ Seamless Stitching  │
     Yes                  No               │ Auto-Continuation   │
      │                   │                └──────────┬──────────┘
      ▼                   ▼                           │
[ Suspend & Emit ]   [ Execute Tool ]                 ▼
  Choice UI Event      Append Output            [ Completed Result ]
  Return Suspended     Loop Next Hop
```

---

## Core Algorithms & Protections

### 1. Robust Tag Unpacker & Stringified JSON Repair (`parse_text_tool_call`)

Local models (like DeepSeek, Qwen, Llama 3) often emit tool calls wrapped in text tags or with stringified JSON inside single arguments. The parser handles all variations:

- **Supported Model Tool Tags:**
  - `<|tool_call|>call:tool_name\n{...}\n<tool_call|>`
  - `call:tool_name{...}`
  - `<function=tool_name>{...}</function>`
  - Standard OpenAI JSON `message.tool_calls`

- **Stringified JSON Unpacking:**
  If a model passes an argument like `question: "{\"options\": [...], \"question\": \"...\"}"`, the parser extracts the outer `{` and `}` boundaries, parses the inner JSON string, and recovers the clean `question` string and `options` array.

- **JSON Auto-Repair:**
  If a JSON string is cut off at the end (e.g. `"question": "..."` without a closing `}`), the parser attempts repair by appending `}` or `"]}` before falling back to regex substring extraction.

### 2. Seamless Stitching Auto-Continuation Loop (`is_incomplete_text`)

When local inference stops mid-sentence due to context bounds or token limits:
1. `is_incomplete_text(&final_content)` checks if the answer ends without standard sentence terminators (`.`, `!`, `?`, `ครับ`, `ค่ะ`, `]`, `}`, `)`).
2. If incomplete, the engine automatically loops up to **3 times**, sending a continuation request:
   ```rust
   ChatMessage {
       role: "user".to_string(),
       content: "Continue exactly where the previous answer ended. Do not repeat text; preserve the language and Markdown structure.".to_string(),
   }
   ```
3. The continuation text is seamlessly stitched to `final_content` before returning to the UI.

### 3. Option Safety Fallback

If `ask_user_clarification` or `ask_user_choice` is invoked but the extracted `options` array is empty, the engine automatically populates 4 standard categories:
1. `เทคโนโลยีและปัญญาประดิษฐ์ (AI / Tech)`
2. `เศรษฐกิจ การเงิน และการลงทุน`
3. `ข่าวสารและเหตุการณ์ปัจจุบัน`
4. `หัวข้ออื่น ๆ (ระบุได้)`

This guarantees that 0-option radio button cards never appear on the user's screen.

### 4. Empty Response Guard (`force_final_answer`)

If the model outputs an empty string `""` after running tool iterations, `agent_loop.rs` triggers `force_final_answer()` to synthesize a text response from the accumulated tool outputs. If still empty, it provides a friendly Thai fallback message instead of writing blank cards to SQLite.
