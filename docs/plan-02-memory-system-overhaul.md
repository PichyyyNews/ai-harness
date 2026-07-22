# แผน 2: ระบบความจำที่ฉลาด — ให้ LLM สรุปเอง ไม่ Hardcode

> **ปัญหาที่แท้จริง:** ไม่ใช่ว่า LLM จำไม่ได้ — แต่เราบังคับให้มัน output JSON schema
> ที่ซับซ้อน ซึ่งมัน (4B quantized) ทำไม่ได้ดี
>
> **แนวทางใหม่:** ให้ LLM ทำในสิ่งที่มันเก่ง — สรุปความ, เข้าใจบริบท, จดจำ
> ในรูปแบบที่เป็นธรรมชาติ (plain text) ไม่ใช่ structured JSON

---

## ทำไม JSON Extraction ถึงผิดแนวทาง

```
สิ่งที่เราสั่ง LLM (4B):
"Extract structured JSON with fields: goals, decisions, facts,
 each fact must have category, content, confidence..."

สิ่งที่ LLM เก่ง:
"สรุปบทสนทนาที่ผ่านมาเป็นภาษาธรรมชาติ"

→ ทำไมเราไม่ให้มันทำสิ่งที่มันเก่ง?
```

### Rule-Based ก็ผิดเหมือนกัน

```
Rule: ถ้าเจอ "ห้าม" → เก็บเป็น constraint
ปัญหา: "ห้ามลืมว่าเมื่อวานฝนตก" → ไม่ใช่ constraint!

Rule: ถ้าเจอ "always" → เก็บเป็น requirement
ปัญหา: "I always forget my password" → ไม่ใช่ requirement!

LLM เข้าใจบริบท — มันรู้ว่าอะไรคือ constraint อะไรไม่ใช่
เราแค่ต้องให้มันบอกเราในรูปแบบที่มัน output ได้ดี
```

---

## แนวทาง: Memory as Tool — ให้ LLM จัดการความจำเอง

### Concept

เหมือนกับ search — ให้ LLM มี "เครื่องมือ" สำหรับจัดการความจำ:

```json
{
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "web_search",
        "description": "Search the web for current information",
        "parameters": { "..." }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "remember",
        "description": "Save important information about the user or conversation for future reference. Use this when the user states a preference, gives a constraint, shares personal info, or makes an important decision.",
        "parameters": {
          "type": "object",
          "properties": {
            "what": {
              "type": "string",
              "description": "What to remember, in natural language"
            },
            "importance": {
              "type": "string",
              "enum": ["rule", "preference", "fact", "context"],
              "description": "How important: rule=must follow always, preference=user likes, fact=info about user, context=conversation context"
            }
          },
          "required": ["what"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "recall",
        "description": "Recall previously saved memories relevant to the current conversation",
        "parameters": {
          "type": "object",
          "properties": {
            "topic": {
              "type": "string",
              "description": "What topic to recall memories about"
            }
          }
        }
      }
    }
  ]
}
```

### ตัวอย่าง Flow

**ผู้ใช้:** "ตอบเป็นภาษาไทยเสมอนะ อย่าใช้ emoji"

**LLM ตอบ:**
```json
{
  "tool_calls": [
    {
      "function": {
        "name": "remember",
        "arguments": "{\"what\": \"ผู้ใช้ต้องการให้ตอบเป็นภาษาไทยเสมอ\", \"importance\": \"rule\"}"
      }
    },
    {
      "function": {
        "name": "remember",
        "arguments": "{\"what\": \"ผู้ใช้ไม่ต้องการให้ใช้ emoji\", \"importance\": \"rule\"}"
      }
    }
  ]
}
```

→ ระบบเก็บลง SQLite → ทุกครั้งที่สนทนาใหม่ inject memories เข้า system prompt

**ข้อดี:**
- LLM **เข้าใจบริบท** — รู้ว่า "ห้ามลืมว่าฝนตก" ไม่ใช่ constraint
- LLM **เลือกเอง** ว่าอะไรควรจำ อะไรไม่ต้อง
- **ทุกภาษา** ทำงานเหมือนกัน
- **ไม่ต้อง hardcode** keyword list

---

## ถ้า Tool Calling ไม่เสถียร → Fallback: Marker Approach

เหมือน search — ใช้ marker ใน stream:

```
System Prompt (เพิ่มเข้าไป):

คุณมีความสามารถจดจำข้อมูลสำคัญ
- ถ้าผู้ใช้บอกกฎ/ข้อจำกัด ให้เขียน <<REMEMBER_RULE: สิ่งที่ต้องจำ>>
- ถ้าผู้ใช้บอกความชอบ ให้เขียน <<REMEMBER_PREF: สิ่งที่ต้องจำ>>
- ถ้าผู้ใช้บอกข้อมูลส่วนตัว ให้เขียน <<REMEMBER_FACT: สิ่งที่ต้องจำ>>

ตัวอย่าง:
- ผู้ใช้: "ผมชื่อสมชาย ทำงานเป็น developer"
  → <<REMEMBER_FACT: ผู้ใช้ชื่อสมชาย อาชีพ developer>>
- ผู้ใช้: "ตอบสั้นๆ ไม่ต้องอธิบายยาว"
  → <<REMEMBER_RULE: ตอบสั้นๆ ไม่ต้องอธิบายยาว>>
```

**การ parse:**
```rust
fn process_memory_markers(response: &str) -> Vec<MemoryItem> {
    let mut items = Vec::new();

    // ง่ายมาก — แค่หา marker ใน text
    for marker in extract_markers(response, "<<REMEMBER_RULE:", ">>") {
        items.push(MemoryItem {
            content: marker,
            tier: MemoryTier::Rule,     // = short-term constraint
            scope: Scope::Session,
        });
    }
    for marker in extract_markers(response, "<<REMEMBER_PREF:", ">>") {
        items.push(MemoryItem {
            content: marker,
            tier: MemoryTier::Preference, // = long-term preference
            scope: Scope::CrossSession,
        });
    }
    for marker in extract_markers(response, "<<REMEMBER_FACT:", ">>") {
        items.push(MemoryItem {
            content: marker,
            tier: MemoryTier::Fact,      // = long-term fact
            scope: Scope::CrossSession,
        });
    }

    // ลบ markers ออกจาก response ที่ส่งให้ผู้ใช้
    items
}
```

---

## 3-Tier Memory ที่ใช้ได้จริง

### Tier 1: Rules (Short-Term Constraints)

**LLM ตัดสินใจ** ว่าอะไรเป็น rule → เก็บลง SQLite → inject ทุก turn

```sql
CREATE TABLE memory_rules (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    content TEXT NOT NULL,     -- plain text จาก LLM ไม่ต้อง parse
    scope TEXT DEFAULT 'session', -- 'session' หรือ 'permanent'
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    active BOOLEAN DEFAULT 1
);
```

**Inject เข้า prompt:**
```
[กฎที่ต้องทำตาม]
- ตอบเป็นภาษาไทยเสมอ
- ไม่ใช้ emoji
- ตอบสั้นๆ กระชับ
```

### Tier 2: Conversation Context (Mid-Term)

**Background Summary** — หลังจบแต่ละ turn ถ้า LLM ว่าง:

```
Prompt (ส่งให้ LLM ตอน idle):
"สรุปประเด็นสำคัญของบทสนทนานี้เป็น 3-5 ข้อสั้นๆ
 เน้นสิ่งที่ยังค้างอยู่ สิ่งที่ตัดสินใจแล้ว และบริบทสำคัญ"

→ LLM ตอบ plain text (ไม่ใช่ JSON):
"- กำลังสร้างระบบ AI chatbot
 - ตัดสินใจใช้ Gemma 4B เป็น model หลัก
 - ยังต้องแก้ปัญหา web search ที่ไม่เสถียร"

→ เก็บ as-is ลง SQLite ไม่ต้อง parse อะไร
```

```sql
CREATE TABLE memory_context (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    summary TEXT NOT NULL,       -- plain text สรุปจาก LLM
    turn_index INTEGER,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

**ถ้า LLM ไม่ว่าง?** → ไม่เป็นไร ใช้ conversation history ตรงๆ
ไม่ต้องมี extractive summary fallback อะไรซับซ้อน

### Tier 3: User Profile (Long-Term)

**LLM สะสมข้อมูลเกี่ยวกับผู้ใช้** ผ่าน `remember` tool/marker → เก็บข้าม session:

```sql
CREATE TABLE memory_facts (
    id INTEGER PRIMARY KEY,
    content TEXT NOT NULL,     -- plain text จาก LLM
    importance TEXT,           -- 'fact', 'preference'
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    last_used DATETIME,       -- อัปเดตทุกครั้งที่ถูก recall
    active BOOLEAN DEFAULT 1
);
```

**Inject เข้า prompt ทุก session ใหม่:**
```
[ข้อมูลที่จำไว้เกี่ยวกับผู้ใช้]
- ชื่อสมชาย อาชีพ developer
- ใช้ Rust และ React เป็นหลัก
- ชอบคำตอบที่กระชับ ตรงประเด็น
- กำลังพัฒนาแอป AI Harness
```

---

## Deduplication — ให้ LLM ช่วยตัดสินใจ

### ปัญหาเดิม: Jaccard similarity สับสน negation

### แนวทางใหม่: ถาม LLM เมื่อ idle

```
เมื่อมี fact ใหม่เข้ามา:
1. ดึง facts ที่มีอยู่ทั้งหมด
2. ถ้ามีมากกว่า 20 entries → ถ้า LLM ว่าง:

Prompt:
"ด้านล่างนี้คือรายการข้อมูลที่จำไว้เกี่ยวกับผู้ใช้
 ช่วยรวมข้อที่ซ้ำกัน ลบข้อที่ขัดแย้งกัน (ใช้ข้อใหม่กว่า)
 และจัดเรียงใหม่:

 1. ชื่อสมชาย
 2. เป็น developer
 3. ชื่อสมชาย อาชีพ developer  ← ซ้ำ
 4. ใช้ Python                  ← เก่า
 5. เปลี่ยนมาใช้ Rust แล้ว      ← ใหม่กว่า"

→ LLM ตอบ:
"1. ชื่อสมชาย อาชีพ developer
 2. ใช้ Rust (เปลี่ยนจาก Python)
"
→ Replace ทั้ง table
```

**ข้อดี:**
- LLM เข้าใจ negation, context, timeline
- ไม่ต้องเขียน dedup algorithm เอง
- ทำตอน idle ไม่ block user

---

## LLM Queue — ใช้ตัวเดียวอย่างฉลาด

### ปัญหา: llama-server ตัวเดียว ต้อง chat + memory + title

### แนวทาง: Priority + Idle Detection

```rust
struct SmartQueue {
    is_chatting: AtomicBool,
    pending_memory_jobs: Mutex<VecDeque<MemoryJob>>,
    idle_threshold: Duration, // 3 วินาทีหลัง user หยุดพิมพ์
}

impl SmartQueue {
    // เรียกจาก chat pipeline — ความสำคัญสูงสุด
    async fn chat(&self, request: ChatRequest) -> ChatResponse {
        self.is_chatting.store(true, Ordering::SeqCst);
        let result = self.llm.generate(request).await;
        self.is_chatting.store(false, Ordering::SeqCst);

        // หลัง chat เสร็จ → ลอง process memory queue
        self.try_process_memory_queue().await;

        result
    }

    // เรียกหลัง chat เสร็จ + user idle
    async fn try_process_memory_queue(&self) {
        // รอ idle threshold
        tokio::time::sleep(self.idle_threshold).await;

        // ถ้า user ยังไม่ได้พิมพ์อะไร → process memory
        if !self.is_chatting.load(Ordering::SeqCst) {
            if let Some(job) = self.pending_memory_jobs.lock().pop_front() {
                self.process_memory_job(job).await;
            }
        }
    }
}
```

**กฎ:**
1. Chat → เสมอมาก่อน ไม่มีข้อยกเว้น
2. Memory → ทำเมื่อ idle เท่านั้น
3. ถ้า user ส่งข้อความใหม่ → cancel memory job
4. Memory ไม่ต้องเสร็จทุก turn → สะสมไปเรื่อยๆ ถ้า idle ก็ค่อยทำ

---

## Memory Injection — เรียบง่ายแต่ทรงพลัง

### ทุก turn ก่อนส่ง prompt เข้า LLM:

```rust
fn build_memory_context(session_id: &str) -> String {
    let mut context = String::new();

    // 1. Rules (ต้องมีเสมอ)
    let rules = db.get_active_rules(session_id);
    if !rules.is_empty() {
        context.push_str("[กฎที่ต้องทำตาม]\n");
        for rule in &rules {
            context.push_str(&format!("- {}\n", rule.content));
        }
        context.push('\n');
    }

    // 2. User facts (ต้องมีเสมอ ถ้ามี)
    let facts = db.get_user_facts();
    if !facts.is_empty() {
        context.push_str("[ข้อมูลเกี่ยวกับผู้ใช้]\n");
        for fact in &facts {
            context.push_str(&format!("- {}\n", fact.content));
        }
        context.push('\n');
    }

    // 3. Conversation summary (ถ้ามี)
    if let Some(summary) = db.get_latest_summary(session_id) {
        context.push_str("[สรุปบทสนทนาที่ผ่านมา]\n");
        context.push_str(&summary);
        context.push('\n');
    }

    context
}
```

**ง่าย, ชัดเจน, ทำงานได้จริง**

---

## Sensitive Data — ให้ LLM ตัดสินเอง

### ปัญหาเดิม: English keyword list สำหรับ filter → ภาษาอื่น bypass ได้

### แนวทางใหม่: เพิ่ม instruction ใน system prompt

```
System prompt (ส่วน memory):
"เมื่อจดจำข้อมูลผู้ใช้ ห้ามจดจำข้อมูลที่อ่อนไหว เช่น:
 - รหัสผ่าน, API key, token
 - ข้อมูลทางการแพทย์/สุขภาพส่วนตัว
 - ข้อมูลทางการเงิน เช่น เลขบัตรเครดิต
 - ความเชื่อทางศาสนา/การเมือง
 ถ้าผู้ใช้บอกข้อมูลเหล่านี้ ให้ตอบได้แต่ห้ามใช้ remember เก็บไว้"
```

→ **LLM ตัดสินเอง** ว่าอะไรควรจำอะไรไม่ควร
→ ทำงานทุกภาษาอัตโนมัติ
→ ไม่ต้อง maintain keyword blacklist

---

## สรุปเปรียบเทียบ

| ด้าน | ระบบเดิม (LLM JSON) | แผนก่อน (Rule-Based) | แผนใหม่ (LLM Tools) |
|------|---------------------|---------------------|---------------------|
| Constraint detection | LLM → JSON ❌ | Keyword match ❌ | LLM → tool call ✅ |
| Memory extraction | LLM → JSON ❌ | Regex patterns ❌ | LLM → remember tool ✅ |
| Multilingual | ❌ JSON prompt = EN only | ❌ ต้อง list ทุกภาษา | ✅ LLM เข้าใจทุกภาษา |
| Deduplication | Jaccard ❌ | Negation check ❌ | LLM consolidate ✅ |
| Dynamic | ✅ แต่ fail | ❌ hardcode | ✅ dynamic + works |
| Sensitive filter | EN keywords ❌ | EN keywords ❌ | LLM instruction ✅ |

---

## ไฟล์ที่ต้องเปลี่ยน

| ไฟล์ | การเปลี่ยนแปลง |
|------|----------------|
| `engine/memory/mod.rs` | ปรับ memory injection ให้เรียบง่าย + plain text |
| `engine/memory/short_term.rs` | รับ memory จาก tool call/marker แทน LLM JSON extract |
| `engine/memory/mid_term.rs` | ใช้ LLM plain text summary แทน JSON schema |
| `engine/memory/long_term.rs` | รับ facts จาก tool call + LLM dedup เมื่อ idle |
| `engine/memory/agent.rs` | ปรับเป็น idle-only queue |
| `commands/engine.rs` | เพิ่ม memory tools เข้า chat request |
| `engine/context_manager.rs` | ปรับ memory budget + injection |
