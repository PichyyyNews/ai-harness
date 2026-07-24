import sys
import os
import argparse
import json
import re

# Add local embedded Aider directory to Python module search path
script_dir = os.path.dirname(os.path.abspath(__file__))
embedded_aider = os.path.join(script_dir, "aider")
if os.path.exists(embedded_aider):
    sys.path.insert(0, embedded_aider)

from aider.io import InputOutput
from aider.models import Model
from aider.coders import Coder

class BridgeInputOutput(InputOutput):
    """Silent InputOutput handler preventing stdout noise during execution."""
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs, pretty=False, yes=True)

    def tool_output(self, status="", bold=False):
        pass

    def tool_error(self, message):
        pass

    def tool_warning(self, message):
        pass

def clean_answer_text(raw_text):
    if not raw_text:
        return ""

    # Remove raw JSON payloads if accidentally attached
    cleaned = re.sub(r'\{"type":.*\}', '', raw_text)

    # Remove SEARCH/REPLACE blocks completely
    cleaned = re.sub(r'<<<<<<< SEARCH[\s\S]*?>>>>>>> REPLACE', '', cleaned)

    # Remove leftover ```python or ``` markdown blocks surrounding search replace
    cleaned = re.sub(r'```[a-zA-Z]*\s*```', '', cleaned)
    cleaned = re.sub(r'\n{3,}', '\n\n', cleaned).strip()

    return cleaned

def parse_aider_output(full_res):
    if not full_res:
        return [], ""

    thinking_lines = []
    answer_lines = []
    current_mode = "answer"

    for line in full_res.splitlines():
        line_str = line.strip()
        if not line_str:
            continue

        if "► THINKING" in line_str or line_str == "THINKING":
            current_mode = "thinking"
            continue
        elif "► ANSWER" in line_str or line_str == "ANSWER":
            current_mode = "answer"
            continue

        # Strip leading ► THINKING or ► ANSWER if attached
        clean = re.sub(r'^►\s*(THINKING|ANSWER)\s*', '', line_str).strip()
        if not clean:
            continue

        if current_mode == "thinking":
            thinking_lines.append(clean)
        else:
            answer_lines.append(clean)

    raw_answer = "\n".join(answer_lines)
    clean_answer = clean_answer_text(raw_answer)

    return thinking_lines, clean_answer

def main():
    parser = argparse.ArgumentParser(description="Aphelion Native Aider Python API Bridge")
    parser.add_argument("--workspace", required=True, help="Target workspace directory")
    parser.add_argument("--prompt", required=True, help="Prompt instruction")
    parser.add_argument("--api-base", default="http://127.0.0.1:8080/v1", help="Local LLM OpenAI API Base URL")
    parser.add_argument("--api-key", default="sk-dummy-key", help="Local LLM API Key")
    parser.add_argument("--model", default="openai/local-model", help="Model identifier")
    parser.add_argument("--no-auto-commits", action="store_true", help="Disable git auto commits")

    args = parser.parse_args()

    os.environ["OPENAI_API_BASE"] = args.api_base
    os.environ["OPENAI_API_KEY"] = args.api_key

    os.makedirs(args.workspace, exist_ok=True)
    os.chdir(args.workspace)

    git_dir = os.path.join(args.workspace, ".git")
    if not os.path.exists(git_dir):
        os.system("git init > nul 2>&1" if os.name == "nt" else "git init > /dev/null 2>&1")

    io = BridgeInputOutput()

    model_name = args.model
    if not (model_name.startswith("openai/") or model_name.startswith("ollama/")):
        model_name = f"openai/{model_name}"

    model = Model(model_name)

    coder = Coder.create(
        main_model=model,
        io=io,
        edit_format="diff",
        auto_commits=not args.no_auto_commits,
    )

    try:
        raw_res = coder.run(args.prompt) or ""
        thinking, content = parse_aider_output(raw_res)

        # Collect edited files
        edited_files = []
        if hasattr(coder, "coder_commit_hashes") and coder.coder_commit_hashes:
            edited_files = list(coder.get_inchat_relative_files())
        elif hasattr(coder, "abs_fnames"):
            edited_files = [coder.get_rel_fname(f) for f in coder.abs_fnames]

        payload = {
            "type": "done",
            "thinking": thinking,
            "content": content,
            "edited_files": edited_files,
        }
        
        # Ensure payload is on a fresh newline
        sys.stdout.write("\n")
        sys.stdout.flush()
        print(json.dumps(payload, ensure_ascii=False), flush=True)

    except Exception as e:
        err_payload = {
            "type": "error",
            "content": str(e),
        }
        sys.stdout.write("\n")
        sys.stdout.flush()
        print(json.dumps(err_payload, ensure_ascii=False), flush=True)
        sys.exit(1)

if __name__ == "__main__":
    main()
