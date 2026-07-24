import sys
import os
import argparse
import json

# Add local embedded Aider directory to Python module search path
script_dir = os.path.dirname(os.path.abspath(__file__))
embedded_aider = os.path.join(script_dir, "aider")
if os.path.exists(embedded_aider):
    sys.path.insert(0, embedded_aider)

from aider.io import InputOutput
from aider.models import Model
from aider.coders import Coder

# CLI noise substrings to suppress completely
NOISE_SUBSTRINGS = (
    "Can't initialize prompt toolkit",
    "Terminal does not support",
    "Tokens:",
    "Has it been deleted",
    "Repo-map:",
    "Git repo:",
    "Model:",
    "Aider v",
    "Commit ",
    "Applied edit",
    "ไม่มีคำสั่ง shell",
    "Summarization failed",
    "summarizer unexpectedly",
    "Warning for",
    "https://aider.chat",
    "Scanning repo",
    "Initial repo scan",
    "You can skip this check",
    "Added .aider*",
)

class BridgeInputOutput(InputOutput):
    """Native Aider InputOutput handler streaming clean JSON events to stdout without prompt-toolkit noise."""
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs, pretty=False, yes=True)

    def tool_output(self, status="", bold=False):
        if status and status.strip():
            clean = status.strip()
            if any(noise in clean for noise in NOISE_SUBSTRINGS):
                return
            print(json.dumps({"type": "stdout", "content": clean}, ensure_ascii=False), flush=True)

    def tool_error(self, message):
        if message and message.strip():
            clean = message.strip()
            if any(noise in clean for noise in NOISE_SUBSTRINGS):
                return
            print(json.dumps({"type": "stderr", "content": clean}, ensure_ascii=False), flush=True)

    def tool_warning(self, message):
        pass

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
        res = coder.run(args.prompt)
        print(json.dumps({"type": "done", "content": res or ""}, ensure_ascii=False), flush=True)
    except Exception as e:
        print(json.dumps({"type": "error", "content": str(e)}, ensure_ascii=False), flush=True)
        sys.exit(1)

if __name__ == "__main__":
    main()
