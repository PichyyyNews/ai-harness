# แผน 3: Dynamic AI Pipeline — สถาปัตยกรรมรวม

> **หลักการ:** ให้ LLM เป็นสมองที่ตัดสินใจทุกอย่าง — ค้นหาเมื่อไหร่, จำอะไร, ตอบอย่างไร
> ระบบรอบข้างเป็นแค่ "มือและเท้า" ที่ทำตามสิ่งที่ LLM สั่ง
> ไม่ hardcode, ไม่ดักด้วย keyword, ไม่ต้อง maintain rule list ใดๆ

---

## สถาปัตยกรรม: LLM as Brain, System as Body

```
┌───────────────────────────────────────────────────────┐
│                    AI Harness                         │
│                                                       │
│  User Message                                         │
│       │                                               │
│       ▼                                               │
│  ┌────────────┐     ┌──────────────────────┐          │
│  │  Prompt    │────▶│  Memory Store        │          │
│  │  Builder   │     │  (SQLite)            │          │
│  │            │◀────│  • rules             │          │
│  │ ประกอบ:    │     │  • facts             │          │
│  │ system +   │     │  • context summary   │          │
│  │ memory +   │     └──────────────────────┘          │
│  │ history +  │                                       │
│  │ tools def  │                                       │
│  └─────┬──────┘                                       │
│        │                                              │
│        ▼                                              │
│  ┌────────────┐                                       │
│  │  LLM       │  ← Gemma 4B via llama-server          │
│  │  (Brain)   │                                       │
│  │            │  LLM ตัดสินใจเอง:                      │
│  │            │  • ตอบเลย? → stream response           │
│  │            │  • ต้อง search? → เรียก tool            │
│  │            │  • ต้องจำ? → เรียก remember tool        │
│  └─────┬──────┘                                       │
│        │                                              │
│   ┌────┴─────────────────────┐                        │
│   │      Tool Executor       │                        │
│   │  (System as Body)        │                        │
│   │                          │                        │
│   │  web_search(query, cat)  │──▶ Search Providers    │
│   │  remember(what, type)    │──▶ SQLite              │
│   │  recall(topic)           │──▶ SQLite              │
│   └────┬─────────────────────┘                        │
│        │                                              │
│        ▼                                              │
│  ┌────────────┐                                       │
│  │  LLM       │  ← ได้ tool results กลับมา             │
│  │  Continue   │    สร้างคำตอบจากข้อมูลจริง             │
│  └─────┬──────┘                                       │
│        │                                              │
│        ▼                                              │
│   Response to User                                    │
│                                                       │
│  ┌────────────┐                                       │
│  │ Background │  ← หลัง response + idle 3s            │
│  │ Memory     │    สรุปบทสนทนา (LLM plain text)       │
│  │ Scan       │    dedup facts (LLM consolidate)      │
│  └────────────┘                                       │
└───────────────────────────────────────────────────────┘
```

---

## Implementation Strategy: 2 ระดับ

เพราะไม่แน่ใจว่า Gemma 4B Q4 จะทำ tool calling ได้ดีแค่ไหน
→ ออกแบบ 2 ระดับ ทดสอบจากระดับสูงก่อน ถ้าไม่เสถียรก็ fallback:

### ระดับ 1: Native Tool Calling (ลองก่อน)

```
ใช้ llama-server /v1/chat/completions + tools parameter
Gemma 4 รองรับ tool use ผ่าน Jinja template

ข้อดี: standard, clean, model ตัดสินใจได้ดี
ข้อเสีย: อาจไม่เสถียรกับ Q4 quantization
```

### ระดับ 2: Streaming Marker (Fallback)

```
ใช้ system prompt instruction + marker detection ใน stream
<<SEARCH: query>> <<REMEMBER_RULE: text>> <<REMEMBER_FACT: text>>

ข้อดี: ทำงานกับทุก model, ง่ายต่อการ parse
ข้อเสีย: ต้องลบ marker ออกจาก response
```

### วิธีเลือก

```rust
enum ToolMode {
    NativeToolCalling,  // ระดับ 1
    StreamingMarkers,   // ระดับ 2
}

// ทดสอบตอน engine start
async fn detect_tool_mode(endpoint: &str) -> ToolMode {
    // ส่ง test request พร้อม tools
    let test = send_tool_test(endpoint).await;

    match test {
        Ok(response) if has_valid_tool_calls(&response) => {
            ToolMode::NativeToolCalling
        }
        _ => {
            // Model ไม่ support หรือ output ไม่ดี → ใช้ markers
            ToolMode::StreamingMarkers
        }
    }
}
```

**Dynamic:** ระบบเลือกเองว่าจะใช้แบบไหน ตามความสามารถของ model ที่ load อยู่

---

## Chat Pipeline — ทีละ Step

### Step 1: Prompt Assembly (< 5ms, ไม่ใช้ LLM)

```rust
async fn build_prompt(
    user_message: &str,
    session_id: &str,
    history: &[Message],
    tool_mode: ToolMode,
) -> ChatRequest {
    // 1. System prompt (คงที่ + memory)
    let system = format!(
        "{}\n\n{}\n\n{}",
        base_system_prompt(),            // identity + capabilities
        time_context(),                   // วันเวลาปัจจุบัน
        build_memory_context(session_id), // rules + facts + summary
    );

    // 2. History (sliding window)
    let fitted_history = fit_to_context(history, context_budget);

    // 3. Tools (ถ้า native mode)
    let tools = match tool_mode {
        ToolMode::NativeToolCalling => Some(define_tools()),
        ToolMode::StreamingMarkers => None, // tools อยู่ใน system prompt แล้ว
    };

    ChatRequest { system, messages: fitted_history, user_message, tools }
}
```

**ไม่มี LLM call ก่อน chat** — assembly ใช้แค่ database query + string formatting

### Step 2: LLM Generation + Tool Handling

```rust
async fn generate_with_tools(request: ChatRequest) -> Response {
    loop {
        let response = llm.generate(request).await; // streaming

        match response.finish_reason {
            // LLM ตอบเลย → เสร็จ
            FinishReason::Stop => return response,

            // LLM เรียก tool → execute แล้ว continue
            FinishReason::ToolCalls => {
                for tool_call in response.tool_calls {
                    let result = execute_tool(tool_call).await;
                    request.messages.push(tool_result_message(result));
                }
                // วน loop ให้ LLM generate ต่อพร้อมผลลัพธ์
                continue;
            }

            // ตอบไม่จบ → auto-continue
            FinishReason::Length => {
                request.messages.push(continue_message());
                continue;
            }
        }
    }
}
```

### Step 3: Tool Execution

```rust
async fn execute_tool(tool_call: ToolCall) -> ToolResult {
    match tool_call.function.name.as_str() {
        "web_search" => {
            let args: SearchArgs = parse_args(&tool_call.function.arguments);
            let results = search_engine.search(&args.query, args.category).await;
            ToolResult::SearchResults(results)
        }

        "remember" => {
            let args: RememberArgs = parse_args(&tool_call.function.arguments);
            memory_store.save(MemoryItem {
                content: args.what,
                tier: match args.importance.as_deref() {
                    Some("rule") => MemoryTier::Rule,
                    Some("preference") => MemoryTier::Preference,
                    _ => MemoryTier::Fact,
                },
                session_id: current_session_id,
            }).await;
            ToolResult::Acknowledged("จดจำแล้ว")
        }

        "recall" => {
            let args: RecallArgs = parse_args(&tool_call.function.arguments);
            let memories = memory_store.search(&args.topic).await;
            ToolResult::Memories(memories)
        }

        _ => ToolResult::Error("Unknown tool")
    }
}
```

### Step 4: Marker-Mode Processing (ถ้าใช้ระดับ 2)

```rust
async fn process_streaming_response(stream: TokenStream) -> ProcessedResponse {
    let mut full_text = String::new();
    let mut memory_items = Vec::new();
    let mut search_needed = None;

    for token in stream {
        full_text.push_str(&token);

        // ตรวจจับ search marker
        if let Some(query) = extract_marker(&full_text, "<<SEARCH:", ">>") {
            search_needed = Some(query);
            break; // หยุด generation → ไป search → regenerate
        }

        // ตรวจจับ memory markers (ไม่ต้องหยุด generation)
        for marker in extract_all_markers(&full_text, "<<REMEMBER_RULE:", ">>") {
            memory_items.push(MemoryItem::rule(marker));
        }
        for marker in extract_all_markers(&full_text, "<<REMEMBER_PREF:", ">>") {
            memory_items.push(MemoryItem::preference(marker));
        }
        for marker in extract_all_markers(&full_text, "<<REMEMBER_FACT:", ">>") {
            memory_items.push(MemoryItem::fact(marker));
        }
    }

    // ถ้าต้อง search → execute search → regenerate พร้อม context
    if let Some(query) = search_needed {
        let results = search_engine.search(&query, None).await;
        return regenerate_with_search(results).await;
    }

    // เก็บ memory items
    for item in memory_items {
        memory_store.save(item).await;
    }

    // ลบ markers ออกจาก text ที่แสดงให้ user
    let clean_text = remove_all_markers(&full_text);
    ProcessedResponse { text: clean_text }
}
```

### Step 5: Background Memory (Idle Only)

```rust
// ทำงานเมื่อ:
// 1. Chat เสร็จแล้ว
// 2. User ไม่ได้พิมพ์อะไร > 3 วินาที
// 3. llama-server ว่าง

async fn background_memory_scan(session_id: &str) {
    // 1. สรุปบทสนทนา (ถ้ามี > 6 turns ที่ยังไม่ได้สรุป)
    let unsummarized = get_unsummarized_turns(session_id);
    if unsummarized.len() >= 6 {
        let prompt = format!(
            "สรุปประเด็นสำคัญของบทสนทนานี้เป็น 3-5 ข้อสั้นๆ:\n\n{}",
            format_turns(&unsummarized)
        );
        let summary = llm.generate_simple(prompt).await;
        // เก็บ as-is ไม่ parse
        db.save_context_summary(session_id, &summary);
    }

    // 2. Consolidate facts (ถ้ามี > 20 facts)
    let facts = db.get_all_facts();
    if facts.len() > 20 {
        let prompt = format!(
            "รายการข้อมูลที่จำไว้มีดังนี้ ช่วยรวมข้อที่ซ้ำกัน \
             ลบข้อที่ล้าสมัย (ใช้ข้อใหม่กว่า) จัดเรียงใหม่:\n\n{}",
            format_facts(&facts)
        );
        let consolidated = llm.generate_simple(prompt).await;
        // Parse simple list format
        db.replace_facts(parse_simple_list(&consolidated));
    }
}
```

---

## Search Provider Infrastructure

ส่วนนี้เป็น "body" ไม่ใช่ "brain" → deterministic logic สมเหตุสมผล:

### Provider Health (Circuit Breaker)

```rust
struct SearchEngine {
    providers: Vec<Box<dyn SearchProvider>>,
    health: HashMap<String, ProviderHealth>,
}

impl SearchEngine {
    async fn search(&self, query: &str, category: Option<&str>) -> Vec<SearchResult> {
        // 1. ลอง dedicated API ก่อน (ถ้า LLM บอก category)
        if let Some(cat) = category {
            if let Some(results) = self.try_dedicated(query, cat).await {
                return results;
            }
        }

        // 2. ลอง general web search ทีละ provider
        for provider in self.available_providers() {
            match provider.search(query).await {
                Ok(r) if !r.is_empty() => {
                    self.health.record_success(provider.name());
                    return r;
                }
                Err(e) => {
                    self.health.record_failure(provider.name());
                    continue;
                }
                _ => continue,
            }
        }

        // 3. ไม่มีผล → empty
        // LLM จะจัดการเอง (ตอบจาก knowledge หรือบอก user)
        vec![]
    }

    fn available_providers(&self) -> Vec<&dyn SearchProvider> {
        self.providers.iter()
            .filter(|p| self.health.get(p.name()).map_or(true, |h| h.is_available()))
            .collect()
    }
}
```

### Provider Chain

```
ลำดับ (deterministic, ไม่ต้อง LLM เลือก):
1. SearXNG (ถ้ามี, เสถียรสุด)
2. DuckDuckGo HTML
3. Brave Search
4. Bing RSS

ถ้าตัวไหนล้ม 3 ครั้งติด → cooldown 5 นาที → ลองตัวถัดไป
```

---

## ตัวอย่าง End-to-End

### ผู้ใช้: "ข่าว AI วันนี้มีอะไรบ้าง"

```
1. Prompt Assembly (5ms)
   - Load memory: rules=[], facts=["ผู้ใช้ชอบภาษาไทย"], summary=null
   - Build system prompt + tools definition

2. LLM Generation
   LLM คิด: "ผู้ใช้ถามข่าว AI วันนี้ → ฉันต้องค้นหาเพราะฉันไม่รู้ข่าวล่าสุด"
   LLM output: tool_call: web_search(query="AI news today", category="news")

3. Tool Execution (1-3s)
   → Google News RSS "AI news today" → 5 headlines

4. LLM Continue (with search results in context)
   LLM ตอบเป็นภาษาไทย (จำ fact ว่าผู้ใช้ชอบภาษาไทย):
   "วันนี้มีข่าว AI ที่น่าสนใจดังนี้:
    1. Google เปิดตัว Gemini 3.0...
    2. OpenAI ประกาศ..."

Total: ~5-8 วินาที (เทียบกับ 15-40 วินาทีระบบเดิม)
```

### ผู้ใช้: "จำไว้นะ ผมชื่อนิว เป็น developer ใช้ Rust"

```
1. Prompt Assembly (5ms)

2. LLM Generation
   LLM คิด: "ผู้ใช้บอกข้อมูลส่วนตัว → ฉันต้องจำไว้"
   LLM output:
   - tool_call: remember(what="ผู้ใช้ชื่อนิว", importance="fact")
   - tool_call: remember(what="อาชีพ developer ใช้ Rust", importance="fact")
   - text: "จำไว้แล้วครับ คุณนิว! ถ้ามีอะไรเกี่ยวกับ Rust ผมช่วยได้เลยนะครับ"

3. Tool Execution (< 1ms)
   → SQLite INSERT 2 facts

4. ครั้งต่อไปที่เริ่ม session ใหม่:
   System prompt จะมี:
   [ข้อมูลเกี่ยวกับผู้ใช้]
   - ผู้ใช้ชื่อนิว
   - อาชีพ developer ใช้ Rust
```

### ผู้ใช้: "อธิบาย quantum computing ให้หน่อย"

```
1. LLM Generation
   LLM คิด: "นี่คือคำถามความรู้ทั่วไป ฉันรู้อยู่แล้ว ไม่ต้อง search"
   LLM output: text (ไม่เรียก tool)
   "Quantum computing คือ..."

→ ไม่เสียเวลา search เลย!
   LLM ฉลาดพอที่จะรู้ว่าไม่ต้อง search
```

---

## ข้อพิจารณาสำคัญ

### 1. Gemma 4B Tool Calling ทำงานได้ไหม?

**ต้องทดสอบก่อน implement:** ส่ง test cases หลายๆ แบบ ดูว่า model:
- เรียก tool ถูกต้องไหม
- arguments ถูก format ไหม
- รู้เมื่อไหร่ไม่ต้องเรียก tool

**ถ้าไม่ดี** → ใช้ streaming marker approach
**ถ้าดีกับ model ใหญ่แต่ไม่ดีกับ model เล็ก** → auto-detect ตาม model

### 2. Token Cost ของ Tool Definitions

Tool definitions ใน prompt ใช้ token (~200-300 tokens)
→ กิน context budget เพิ่ม
→ แต่ **ประหยัดกว่า** การเรียก LLM 3-4 ครั้งแยก

### 3. Latency ของ Tool Loop

```
Tool calling flow:
LLM generate (2-3s) → tool execute (1-3s) → LLM continue (3-5s)
Total: ~6-11s

เทียบกับระบบเดิม:
Tier 0 (2s) → Tier 1 (2s) → plan (2s) → search (3s) → LLM chat (5s)
Total: ~14s+
```

→ Tool calling **เร็วกว่า** แม้จะมี round-trip

### 4. Model ต่างกัน = Behavior ต่างกัน

ไม่เป็นไร! นี่คือ "dynamic":
- Model ใหญ่ → เรียก tool แม่นยำกว่า
- Model เล็ก → อาจ search เกินจำเป็นบ้าง (ยอมรับได้)
- Model ใหม่ในอนาคต → ทำงานดีขึ้นโดยไม่ต้องแก้โค้ด

---

## Priority Implementation Order

### Week 1: ทำให้ทำงานได้

| # | งาน | เป้าหมาย |
|---|------|---------|
| 1 | ทดสอบ Gemma 4B tool calling | รู้ว่าใช้ native หรือ marker |
| 2 | Implement tool executor framework | รองรับ web_search + remember + recall |
| 3 | Implement memory store (simplified) | SQLite: rules, facts, summaries |
| 4 | Implement search provider chain + health | เสถียร, มี fallback |
| 5 | Wire everything together | End-to-end flow ทำงานได้ |

### Week 2: ทำให้ดี

| # | งาน | เป้าหมาย |
|---|------|---------|
| 6 | Background memory scan | สรุป + consolidate เมื่อ idle |
| 7 | LLM priority queue | Memory ไม่แย่ง chat |
| 8 | Cross-session memory | Facts ข้ามจาก session เก่า |
| 9 | Pipeline observability | User เห็น search/memory status |
| 10 | Streaming marker fallback | สำหรับ model ที่ไม่รองรับ tools |

### Week 3: ทำให้ยอดเยี่ยม

| # | งาน | เป้าหมาย |
|---|------|---------|
| 11 | SearXNG integration guide | Search เสถียรสุด |
| 12 | Result caching | ลด API calls ซ้ำ |
| 13 | User memory management UI | ดู/แก้/ลบ memory ได้ |
| 14 | Auto tool-mode detection | เลือก native/marker ตาม model |
| 15 | Multi-model testing | ทดสอบกับ Qwen, Phi, Llama |

---

## หลักการที่ต้องจำ

> **"ให้ LLM ทำในสิ่งที่ LLM เก่ง — เข้าใจภาษา ตัดสินใจ สรุปความ"**
> **"ให้โค้ดทำในสิ่งที่โค้ดเก่ง — เรียก API, เก็บ database, จัดการ queue"**
>
> อย่าเอาโค้ดไปทำงานของ LLM (hardcode keyword matching)
> อย่าเอา LLM ไปทำงานของโค้ด (สร้าง JSON schema ซับซ้อน)

สิ่งที่ hardcode ได้ (infrastructure logic):
- ✅ Provider chain order
- ✅ Circuit breaker threshold
- ✅ Context budget allocation
- ✅ Queue priority

สิ่งที่ต้องให้ LLM ตัดสิน (language understanding):
- ✅ ต้อง search ไหม + search อะไร
- ✅ ต้องจำอะไร + ความสำคัญระดับไหน
- ✅ ตอบภาษาอะไร + ยาวแค่ไหน
- ✅ ข้อมูลไหนอ่อนไหว
