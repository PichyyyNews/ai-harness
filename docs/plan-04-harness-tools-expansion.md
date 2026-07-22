# แผน 4: AI Harness Native Tools Expansion — ยกระดับ AI ให้เก่งและทรงพลังยิ่งขึ้น

> **เป้าหมาย:** เปลี่ยน AI Harness จาก Chatbot ปกติให้กลายเป็น **AI Desktop Agent** ที่ทำงานร่วมกับระบบ Harness ได้อย่างสมบูรณ์แบบ — สามารถดึงประวัติแชตเก่ามาดู, ค้นหาบทสนทนาข้าม session, ตรวจสอบสเป็กเครื่อง/VRAM, สลับ/จัดการโมเดล GGUF, และอ่านไฟล์งานได้

---

## 🚀 เครื่องมือใหม่ 5 ชุดที่จะเพิ่มเข้ามา (Harness Native Tools)

### 1. 📚 Session & Chat History Tools (ค้นหาและอ่านประวัติการคุยเก่า)

ให้ AI ย้อนกลับไปค้นหาและอ่านบทสนทนาในอดีตได้ข้าม session เมื่อผู้ใช้ถามถึงสิ่งที่เคยคุยไว้สัปดาห์ก่อน หรือย้อนความหลัง

```json
{
  "tools": [
    {
      "name": "search_chat_history",
      "description": "Search past chat sessions and messages for specific keywords, topics, or historical answers across all user sessions",
      "parameters": {
        "type": "object",
        "properties": {
          "query": { "type": "string", "description": "Search keyword or topic" },
          "limit": { "type": "integer", "description": "Maximum number of sessions to return (default 5)" }
        },
        "required": ["query"]
      }
    },
    {
      "name": "get_session_details",
      "description": "Retrieve full transcript of a specific past chat session by session ID or relative time (e.g. 'previous_session', 'last_week')",
      "parameters": {
        "type": "object",
        "properties": {
          "session_id": { "type": "string", "description": "The unique ID of the session" }
        },
        "required": ["session_id"]
      }
    }
  ]
}
```

**ตัวอย่างการใช้งาน:**
- **ผู้ใช้:** *"เมื่อวานเราคุยกันเรื่องโค้ด Rust ไว้ตรงไหนนะ"*
- **LLM:** เรียก `search_chat_history("Rust")` → ดึงบทสนทนาเมื่อวาน → อ่านสรุปและต่อยอดให้ผู้ใช้ทันที

---

### 2. 🤖 Model & Engine Management Tools (ควบคุมและสลับโมเดล GGUF)

ให้ AI ตรวจสอบโมเดลที่ติดตั้งในเครื่อง ค้นหาโมเดลใหม่บน Hugging Face หรือแนะนำการสลับโมเดลตามภารกิจ

```json
{
  "tools": [
    {
      "name": "list_installed_models",
      "description": "List all GGUF model files currently downloaded and installed on the local machine",
      "parameters": { "type": "object", "properties": {} }
    },
    {
      "name": "search_huggingface_models",
      "description": "Search Hugging Face catalog for open GGUF models by keyword or architecture (e.g. 'gemma', 'qwen', 'coder')",
      "parameters": {
        "type": "object",
        "properties": {
          "query": { "type": "string", "description": "Search query for GGUF models" }
        },
        "required": ["query"]
      }
    }
  ]
}
```

**ตัวอย่างการใช้งาน:**
- **ผู้ใช้:** *"ตอนนี้ในเครื่องมีโมเดลอะไรบ้าง อยากได้โมเดลเขียนโค้ดดีๆ"*
- **LLM:** เรียก `list_installed_models()` + `search_huggingface_models("coder GGUF")` → แนะนำโมเดลพร้อมขนาดและ VRAM ที่ต้องใช้

---

### 3. 💻 Hardware & System Diagnostics Tools (ตรวจสอบ VRAM, RAM, Context)

ให้ AI เช็กสเป็กเครื่อง VRAM ที่เหลือ และโทเค็นที่ใช้อยู่ได้ เพื่อปรับแต่งคำตอบให้เหมาะกับทรัพยากรเครื่องของผู้ใช้

```json
{
  "tools": [
    {
      "name": "get_system_status",
      "description": "Check current system resources including free VRAM, RAM usage, active backend (CUDA/CPU), and llama-server health",
      "parameters": { "type": "object", "properties": {} }
    },
    {
      "name": "get_context_usage",
      "description": "Inspect current conversation token usage against maximum context window size",
      "parameters": { "type": "object", "properties": {} }
    }
  ]
}
```

**ตัวอย่างการใช้งาน:**
- **ผู้ใช้:** *"ทำไมตอบช้าจัง VRAM พอไหมตอนนี้"*
- **LLM:** เรียก `get_system_status()` → ตอบ *"ตอนนี้ใช้ CUDA อยู่ครับ VRAM เหลือ 4.2 GB อุณหภูมิ GPU ปกติครับ"*

---

### 4. 📁 Workspace & File Access Tools (อ่านและตรวจไฟล์งานท้องถิ่น)

ให้ AI อ่านไฟล์ข้อความ โค้ด หรือเอกสารในโฟลเดอร์ Workspace ของผู้ใช้ เพื่อช่วยวิเคราะห์โค้ด สรุปเอกสาร หรือเขียนไฟล์ใหม่

```json
{
  "tools": [
    {
      "name": "read_workspace_file",
      "description": "Read content of a local text or code file within the workspace directory",
      "parameters": {
        "type": "object",
        "properties": {
          "relative_path": { "type": "string", "description": "Relative path to file within workspace" }
        },
        "required": ["relative_path"]
      }
    },
    {
      "name": "list_workspace_files",
      "description": "List files and subdirectories in the workspace",
      "parameters": {
        "type": "object",
        "properties": {
          "directory": { "type": "string", "description": "Subdirectory path, defaults to root" }
        }
      }
    }
  ]
}
```

**ตัวอย่างการใช้งาน:**
- **ผู้ใช้:** *"ช่วยรีวิวโค้ดใน src/App.tsx หน่อย"*
- **LLM:** เรียก `read_workspace_file("src/App.tsx")` → อ่านโค้ดและวิเคราะห์พร้อมแนะนำจุดปรับปรุง

---

### 5. 🧮 Sandbox Calculator / Code Evaluator (คำนวณและประมวลผลแม่นยำ 100%)

แก้ปัญหา LLM คำนวณคณิตศาสตร์ซับซ้อนหรือคำนวณวันที่ผิดพลาด โดยให้ AI ส่งนิพจน์คณิตศาสตร์เข้า Sandbox Evaluator

```json
{
  "tools": [
    {
      "name": "evaluate_expression",
      "description": "Safely evaluate mathematical expressions, unit conversions, or date/time calculations with 100% accuracy",
      "parameters": {
        "type": "object",
        "properties": {
          "expression": { "type": "string", "description": "Math or logic expression (e.g. '((345 * 12.5) / 100) * 1.07', 'date_diff(2026-01-01, 2026-07-22)')" }
        },
        "required": ["expression"]
      }
    }
  ]
}
```

---

## 🏗️ การออกแบบสถาปัตยกรรม Backend (Tauri Rust)

### 1. Tool Registry Pattern (`src-tauri/src/tools/`)

สร้างโมดูล `tools/` ใน Rust เพื่อรวบรวม Native Tools ทั้งหมด:

```text
src-tauri/src/tools/
├── mod.rs                # Registry และ Dispatcher หลัก
├── history.rs            # ค้นหาและดึง SQLite chat sessions
├── models.rs             # ค้นหาและตรวจสอบ GGUF models
├── system.rs             # ตรวจสอบ VRAM / Hardware Profile
├── workspace.rs          # อ่านและสำรวจไฟล์ใน Workspace
└── evaluator.rs          # คำนวณคณิตศาสตร์และนิพจน์
```

### 2. Dispatcher Logic (`tools/mod.rs`)

```rust
pub fn execute_tool(app: &AppHandle, name: &str, args: &str) -> Result<String, String> {
    match name {
        "search_chat_history" => history::search(app, args),
        "get_session_details" => history::get_details(app, args),
        "list_installed_models" => models::list_installed(app),
        "search_huggingface_models" => models::search_hf(args),
        "get_system_status" => system::status(app),
        "read_workspace_file" => workspace::read_file(app, args),
        "evaluate_expression" => evaluator::eval(args),
        _ => Err(format!("Unknown tool: {name}")),
    }
}
```

---

## 📋 แผนการพัฒนา (Implementation Roadmap)

### Phase 1: Chat History & Memory Tools (1-2 วัน)
- Implement `search_chat_history` & `get_session_details` ดึงข้อมูลจาก SQLite `harness.db`
- เชื่อมต่อกับ Chat Stream ให้ผู้ใช้ย้อนถามประวัติเก่าได้ทันที

### Phase 2: System Status & Model Tools (1-2 วัน)
- Implement `get_system_status` อ่าน VRAM / RAM ล่าสุดจาก `engine::hardware`
- Implement `list_installed_models` อ่านรายชื่อไฟล์ GGUF ใน folder

### Phase 3: Workspace File Reader & Evaluator (2-3 วัน)
- Implement `read_workspace_file` ปลอดภัยเฉพาะในวง Workspace
- Implement `evaluate_expression` สำหรับคำนวณคณิตศาสตร์และวันที่

---

## 🎯 ประโยชน์ที่ผู้ใช้จะได้รับ
1. **จำประวัติเก่าได้ไม่จำกัด**: ย้อนดูสิ่งที่คุยไว้ใน Session เก่าๆ ได้ทุกเมื่อ
2. **เข้าใจเครื่องตัวเอง**: รู้ทันทีว่าโมเดลไหนใช้ VRAM เท่าไหร่ และเครื่องไหวไหม
3. **ช่วยงานโค้ด/เอกสารได้จริง**: อ่านไฟล์ในโปรเจกต์มาวิเคราะห์ได้โดยไม่ต้อง Copy-Paste
4. **คำนวณเป๊ะ 100%**: ไม่โดนปัญหา LLM หลอนตัวเลข
