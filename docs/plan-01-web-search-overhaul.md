# แผน 1: ระบบค้นหาที่ฉลาด — ให้ LLM ตัดสินใจเอง ไม่ Hardcode

> **ปัญหาที่แท้จริง:** ไม่ใช่ว่า LLM ไม่ฉลาดพอ — แต่เราใช้มันผิดวิธี
> เราสั่งให้ LLM output JSON → มันทำไม่ได้ → แล้วเราก็จะเปลี่ยนไปใช้ rule-based
> → ก็กลายเป็น hardcode อีกแบบ ซึ่งไม่มีทาง cover ทุกภาษา ทุกบริบท
>
> **แนวทางใหม่:** ใช้ความฉลาดของ LLM จริงๆ แต่ใช้ให้ถูกวิธี

---

## ทำไม Rule-Based ถึงผิด

```
ผู้ใช้พิมพ์: "วันนี้มีอะไรน่าสนใจบ้าง"
→ Rule-based ต้องรู้ว่า "วันนี้" = temporal keyword ✓
→ แต่ถ้าพิมพ์ "เมื่อกี้เห็นข่าวอะไรมา" → ไม่ match

ผู้ใช้พิมพ์: "btc ราคาเท่าไหร่"
→ Rule-based ต้องรู้ว่า "btc" = crypto ✓
→ แต่ถ้าพิมพ์ "บิทคอยน์ตอนนี้ขึ้นหรือลง" → อาจ miss

ผู้ใช้พิมพ์: "apa cuaca hari ini di Jakarta?"
→ Rule-based ต้องรู้ภาษาอินโดด้วย? → ไม่มีทาง cover ได้

LLM เข้าใจทุกอย่างข้างบนโดยธรรมชาติ — เราแค่ต้องให้มันบอกเราว่าต้องทำอะไร
```

---

## แนวทาง: Tool Calling — ให้ LLM เรียกใช้เครื่องมือเอง

### Concept

แทนที่จะ:
```
User → [LLM classify] → [LLM pick provider] → [search] → [LLM chat]
        ↑ ผิดตรงนี้        ↑ ผิดตรงนี้
        JSON ผิด format    JSON ผิด format
```

เปลี่ยนเป็น:
```
User → [LLM chat + tools] → LLM ตัดสินใจเอง
                              ├── ถ้าต้อง search → เรียก tool
                              ├── ถ้าไม่ต้อง     → ตอบเลย
                              └── ทุกภาษา ทุกบริบท ทำงานเหมือนกัน
```

### llama-server รองรับ Tool Calling อยู่แล้ว

ระบบปัจจุบันใช้ `--jinja` flag อยู่แล้ว → Gemma 4 รองรับ tool use ผ่าน chat template

```json
// POST /v1/chat/completions
{
  "messages": [
    {"role": "system", "content": "You are a helpful assistant..."},
    {"role": "user", "content": "ราคา bitcoin ตอนนี้เท่าไหร่"}
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "web_search",
        "description": "Search the web for current information, news, prices, weather, or any real-time data",
        "parameters": {
          "type": "object",
          "properties": {
            "query": {
              "type": "string",
              "description": "The search query"
            },
            "category": {
              "type": "string",
              "enum": ["general", "news", "weather", "crypto", "currency", "wiki", "academic"],
              "description": "Category hint to route to the best source"
            }
          },
          "required": ["query"]
        }
      }
    }
  ],
  "tool_choice": "auto"
}
```

**LLM จะตอบ:**
```json
{
  "choices": [{
    "message": {
      "role": "assistant",
      "tool_calls": [{
        "function": {
          "name": "web_search",
          "arguments": "{\"query\": \"bitcoin price today\", \"category\": \"crypto\"}"
        }
      }]
    }
  }]
}
```

→ ระบบรับ tool call → เรียก API → ส่งผลกลับ → LLM ตอบจากข้อมูลจริง

### ข้อดีที่ได้ทันที

| ด้าน | ทำไมดี |
|------|--------|
| **ทุกภาษา** | LLM เข้าใจ "อากาศวันนี้", "what's the weather", "今日の天気" เหมือนกัน |
| **ทุกบริบท** | LLM รู้ว่า "btc", "บิทคอยน์", "Bitcoin" คือสิ่งเดียวกัน |
| **Dynamic** | ไม่ต้องเพิ่ม keyword list เมื่อมีภาษาใหม่ |
| **ฉลาด** | LLM รู้ว่า "อธิบาย quantum computing" ไม่ต้อง search แต่ "quantum computing breakthrough 2026" ต้อง |
| **เร็วขึ้น** | จาก 3-4 LLM calls → 1 call ที่ทำทุกอย่าง |

---

## ถ้า Gemma 4B Tool Calling ไม่เสถียร → Fallback: Streaming Marker

ถ้า tool calling ทำงานไม่ดีกับ Gemma 4B Q4 (เป็นไปได้เพราะ quantized) ใช้แนวทางสำรอง:

### แนวทาง: ใส่ instruction ใน system prompt ให้ LLM ใช้ marker

```
System Prompt:
คุณมีเครื่องมือค้นหาข้อมูลได้ ถ้าต้องการข้อมูลจากอินเทอร์เน็ต
ให้เขียน <<SEARCH: คำค้นหา>> ไว้ในคำตอบ ระบบจะค้นหาให้อัตโนมัติ
ถ้าไม่ต้องค้นหา ให้ตอบปกติ

ตัวอย่าง:
- ผู้ใช้ถาม "ข่าววันนี้" → <<SEARCH: latest news today>>
- ผู้ใช้ถาม "อธิบาย recursion" → ตอบเลยไม่ต้องค้นหา
```

**วิธีทำงาน:**
```rust
// ขณะ streaming tokens จาก LLM
fn process_stream(token: &str, buffer: &mut String) {
    buffer.push_str(token);

    // ตรวจจับ marker ใน stream
    if let Some(search_query) = extract_marker(buffer, "<<SEARCH:", ">>") {
        // 1. หยุด stream ชั่วคราว
        // 2. ทำ search ด้วย query ที่ LLM เลือก
        // 3. Inject ผลลัพธ์กลับเข้า prompt
        // 4. ให้ LLM generate ต่อพร้อมข้อมูลจริง

        let results = execute_search(&search_query).await;
        inject_and_regenerate(results);
    }
}
```

**ข้อดีของ marker approach:**
- ใช้ได้กับทุก model (ไม่ต้องรองรับ tool calling)
- LLM ตัดสินใจเอง (dynamic, ไม่ hardcode)
- ง่ายต่อการ parse (แค่หา `<<SEARCH:` ... `>>`)
- LLM เลือก query เอง — ฉลาดกว่า keyword matching

---

## Search Execution Layer (ไม่เปลี่ยน concept, แก้ที่ reliability)

เมื่อ LLM ตัดสินใจว่าต้อง search แล้ว ระบบ execution ยังใช้แนวคิดเดิม แต่ทำให้เสถียร:

### Provider Chain with Health Tracking

```rust
// ลำดับการค้นหา — ไม่ต้อง LLM เลือก (ตรงนี้ deterministic ได้)
async fn execute_search(query: &str, category: &str) -> SearchResults {
    // 1. ถ้ามี category hint จาก LLM → ลอง dedicated API ก่อน
    if let Some(result) = try_dedicated_api(query, category).await {
        return result; // เร็ว < 1s, เสถียร
    }

    // 2. General web search — ลองทีละ provider จนได้
    for provider in health_registry.available_providers() {
        match provider.search(query).await {
            Ok(results) if results.len() > 0 => {
                health_registry.record_success(provider);
                return results;
            }
            Ok(_) => continue, // ได้ 0 results → ลองตัวถัดไป
            Err(e) => {
                health_registry.record_failure(provider, e);
                continue;
            }
        }
    }

    // 3. ไม่มีผล → return empty (LLM จะจัดการเอง)
    SearchResults::empty()
}
```

### Health Registry (ตรงนี้ deterministic เหมาะสม)

```rust
// Provider health tracking — ไม่ใช่ classification
// ตรงนี้ rule-based สมเหตุสมผล เพราะเป็น infrastructure logic ไม่ใช่ language understanding
struct ProviderHealth {
    failures: u32,
    cooldown_until: Option<Instant>,
}

impl ProviderHealth {
    fn is_available(&self) -> bool {
        self.cooldown_until
            .map(|t| Instant::now() > t)
            .unwrap_or(true)
    }
}
```

---

## Dedicated APIs — เมื่อ LLM บอก category

LLM ส่ง category hint มาด้วย (ถ้ามี) → route ไป API เฉพาะทาง:

```
category: "weather"  → Open-Meteo API (มีแล้ว)
category: "crypto"   → CoinGecko API (มีแล้ว)
category: "currency" → ExchangeRate API (มีแล้ว)
category: "news"     → Google News RSS (มีแล้ว)
category: "wiki"     → Wikipedia REST (มีแล้ว)
category: "academic" → arXiv / Semantic Scholar (มีแล้ว)
category: "general"  → Web search providers
ไม่มี category       → Web search providers
```

**LLM เลือก category เอง** — ไม่ต้องเราเขียนกฎ:
- "ราคาทอง" → LLM อาจส่ง `category: "currency"` หรือ `"general"`
- "สภาพอากาศเชียงใหม่" → LLM ส่ง `category: "weather"`
- ผิดก็ไม่เป็นไร → fallback ไป general search

---

## สรุปสิ่งที่เปลี่ยนจากแผนเดิม

| เดิม (Rule-Based ❌) | ใหม่ (LLM-Driven ✅) |
|----------------------|---------------------|
| เขียน keyword list ทุกภาษา | LLM เข้าใจทุกภาษาอยู่แล้ว |
| Pattern match ดัก intent | LLM ตัดสินใจเองผ่าน tool call |
| LLM call แยก 3-4 ครั้ง | 1 call เดียวทำทุกอย่าง |
| LLM ต้อง output JSON | LLM ใช้ tool calling หรือ marker (ธรรมชาติกว่า) |
| เพิ่มภาษาใหม่ = แก้โค้ด | เพิ่มภาษาใหม่ = ไม่ต้องทำอะไร |
| อุดรอยรั่วเรื่อยๆ | Dynamic ตั้งแต่ต้น |

---

## ไฟล์ที่ต้องเปลี่ยน

| ไฟล์ | การเปลี่ยนแปลง |
|------|----------------|
| `commands/engine.rs` | ปรับ chat pipeline: ส่ง tools definition ไปกับ request |
| `engine/runtime.rs` | เพิ่ม tool calling support ใน request format |
| `web_search/orchestrator.rs` | รับ tool call → execute → return results |
| `web_search/planner.rs` | **ลบ** LLM-based planning ออก → ใช้ category จาก tool call |
| `web_search/query.rs` | **ลบ** routing_decision → LLM ตัดสินใจแทน |
| `web_search/mod.rs` | ปรับ interface ให้รับ tool call format |
| `web_search/health.rs` | **ใหม่** — circuit breaker (infra logic, rule-based สมเหตุสมผล) |
