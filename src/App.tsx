import { Children, isValidElement, type ComponentPropsWithoutRef, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { open as openShellUrl } from "@tauri-apps/plugin-shell";
import { ArrowDown, ArrowUp, CaretDown, CaretLeft, CaretRight, CaretUp, ChatCircleDots, Check, Code, Copy, DownloadSimple, FolderSimple, Gear, House, MagnifyingGlass, Minus, Palette, PencilSimple, Play, Plus, SidebarSimple, Square, Stop, Trash, User, UserPlus, X } from "@phosphor-icons/react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeKatex from "rehype-katex";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import "katex/dist/katex.min.css";
import logoUrl from "../logo.svg";
import { Badge, Button, Card, DiffPreviewCard, InteractiveChoiceBox, MorphingInfinity, Textarea, TextShimmerWave, Toast, WorkspaceFolderPicker } from "./components";
import type { CatalogModel, DownloadProgress, InstalledModel, ModelFile, RetrievalTraceEntry, SessionDetail, SessionSummary, WebSource } from "./types";
import { streamLocalChat, type ChatMessage, type InteractionOption } from "./lib/local-chat";
import styles from "./App.module.css";

type Screen = "picker" | "chat";
type MenuName = "File" | "Edit" | "View" | "Help";
type WindowAction = "minimize" | "maximize" | "close";
type ConversationMessage = ChatMessage & { process?: string[]; sources?: WebSource[]; retrievalTrace?: RetrievalTraceEntry[]; isQueued?: boolean };
interface PendingChoice { id: string; question: string; options: InteractionOption[]; }

const formatBytes = (bytes?: number) => {
  if (!bytes) return "Size unavailable";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(index < 2 ? 0 : 1)} ${units[index]}`;
};

const preferredFile = (model?: CatalogModel): ModelFile | undefined =>
  model?.files.find((file) => !file.name.toLowerCase().includes("mmproj")) ?? model?.files[0];

const formatSessionTime = (timestamp: string) => {
  const elapsed = Date.now() - Number(timestamp);
  if (!Number.isFinite(elapsed) || elapsed < 60_000) return "now";
  if (elapsed < 3_600_000) return `${Math.floor(elapsed / 60_000)}m`;
  if (elapsed < 86_400_000) return `${Math.floor(elapsed / 3_600_000)}h`;
  return `${Math.floor(elapsed / 86_400_000)}d`;
};

export default function App() {
  const [screen, setScreen] = useState<Screen>("picker");
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [activeMenu, setActiveMenu] = useState<MenuName | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [models, setModels] = useState<CatalogModel[]>([]);
  const [selectedModel, setSelectedModel] = useState<CatalogModel | null>(null);
  const [selectedFile, setSelectedFile] = useState("");
  const [installedModels, setInstalledModels] = useState<InstalledModel[]>([]);
  const [activeModel, setActiveModel] = useState<InstalledModel | null>(null);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [detailsLoading, setDetailsLoading] = useState(false);
  const [download, setDownload] = useState<DownloadProgress | null>(null);
  const [newChatRequest, setNewChatRequest] = useState(0);

  const refreshInstalled = useCallback(async () => {
    const installed = await invoke<InstalledModel[]>("list_installed_models");
    setInstalledModels(installed);
    return installed;
  }, []);

  const searchModels = useCallback(async (query = "") => {
    setCatalogLoading(true);
    try {
      const results = await invoke<CatalogModel[]>("search_models", { query });
      setModels(results);
      if (!selectedModel && results[0]) {
        setSelectedModel(results[0]);
        setSelectedFile(preferredFile(results[0])?.name ?? "");
      }
    } catch (error) {
      setToast(`Could not load the Hugging Face catalog: ${String(error)}`);
    } finally {
      setCatalogLoading(false);
    }
  }, [selectedModel]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const installed = await refreshInstalled();
        if (!cancelled && installed[0]) {
          setActiveModel(installed[0]);
          setScreen("chat");
        }
      } catch (error) {
        if (!cancelled) setToast(`Could not read installed models: ${String(error)}`);
      }
      if (!cancelled) await searchModels();
    })();
    return () => { cancelled = true; };
  }, [refreshInstalled, searchModels]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<DownloadProgress>("model-download-progress", (event) => setDownload(event.payload)).then((handler) => { unlisten = handler; });
    return () => unlisten?.();
  }, []);

  const installedSelection = useMemo(
    () => selectedModel && selectedFile ? installedModels.find((model) => model.repoId === selectedModel.id && model.fileName === selectedFile) : undefined,
    [installedModels, selectedFile, selectedModel],
  );

  const chooseModel = async (model: CatalogModel) => {
    setSelectedModel(model);
    setSelectedFile(preferredFile(model)?.name ?? "");
    setDetailsLoading(true);
    try {
      const details = await invoke<CatalogModel>("get_model_details", { repoId: model.id });
      setSelectedModel(details);
      setSelectedFile((current) => details.files.some((file) => file.name === current) ? current : (preferredFile(details)?.name ?? ""));
    } catch (error) {
      setToast(`Could not load the model files: ${String(error)}`);
    } finally {
      setDetailsLoading(false);
    }
  };

  const installSelected = async () => {
    if (!selectedModel || !selectedFile) return;
    setDownload({ repoId: selectedModel.id, fileName: selectedFile, stage: "downloading", downloadedBytes: 0, totalBytes: undefined, percent: 0 });
    try {
      const installed = await invoke<InstalledModel>("install_model", { request: { repoId: selectedModel.id, fileName: selectedFile } });
      setInstalledModels(await refreshInstalled());
      setActiveModel(installed);
      setDownload(null);
      setScreen("chat");
      setToast("Model verified and ready to use locally.");
    } catch (error) {
      setDownload(null);
      setToast(`Installation did not complete: ${String(error)}`);
    }
  };

  return (
    <main className={styles.app} data-theme={theme}>
      <DesktopMenuBar
        activeMenu={activeMenu}
        sidebarCollapsed={sidebarCollapsed}
        onToggleSidebar={() => setSidebarCollapsed((prev) => !prev)}
        onMenuChange={setActiveMenu}
        onWindowError={setToast}
        onAction={(action) => {
          setActiveMenu(null);
          if (action === "model-picker") setScreen("picker");
          if (action === "workspace") activeModel ? setScreen("chat") : setToast("Install a model before opening the chat workspace.");
          if (action === "light") setTheme("light");
          if (action === "dark") setTheme("dark");
          if (action === "new-chat") {
            if (activeModel) {
              setScreen("chat");
              setNewChatRequest((current) => current + 1);
            } else setToast("Choose and install a model first.");
          }
          if (action === "about") setToast("AI Harness — a local-first desktop workspace for open models.");
          if (action === "shortcuts") setToast("Keyboard shortcuts will be added with the chat milestone.");
        }}
      />

      <div className={`${styles.contentArea} ${screen === "chat" ? styles.chatContentArea : ""}`}>
        {screen === "picker" ? (
          <ModelPicker
            catalogLoading={catalogLoading}
            detailsLoading={detailsLoading}
            models={models}
            selectedModel={selectedModel}
            selectedFile={selectedFile}
            installed={installedSelection}
            download={download}
            onSearch={searchModels}
            onSelect={chooseModel}
            onFileChange={setSelectedFile}
            onInstall={installSelected}
            onOpenInstalled={() => {
              if (installedSelection) {
                setActiveModel(installedSelection);
                setScreen("chat");
              }
            }}
          />
        ) : activeModel ? (
          <ChatWorkspace model={activeModel} newChatRequest={newChatRequest} sidebarCollapsed={sidebarCollapsed} onBack={() => setScreen("picker")} onNotify={setToast} />
        ) : null}
      </div>

      {toast && <div className={styles.toastRegion}><Toast message={toast} type="info" onClose={() => setToast(null)} /></div>}
    </main>
  );
}

async function controlWindow(action: WindowAction) {
  const command = action === "minimize" ? "minimize_window" : action === "maximize" ? "toggle_maximize_window" : "close_window";
  await invoke(command);
}

async function startWindowDrag() { await invoke("start_window_dragging"); }

interface DesktopMenuBarProps {
  activeMenu: MenuName | null;
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
  onMenuChange: (menu: MenuName | null) => void;
  onWindowError: (message: string) => void;
  onAction: (action: "model-picker" | "workspace" | "light" | "dark" | "new-chat" | "about" | "shortcuts") => void;
}

function DesktopMenuBar({ activeMenu, sidebarCollapsed, onToggleSidebar, onMenuChange, onWindowError, onAction }: DesktopMenuBarProps) {
  const reportWindowError = (label: string, error: unknown) => onWindowError(`${label} failed: ${error instanceof Error ? error.message : String(error)}`);
  const menus: Record<MenuName, Array<{ label: string; action: Parameters<DesktopMenuBarProps["onAction"]>[0] }>> = {
    File: [{ label: "New chat", action: "new-chat" }, { label: "Choose model", action: "model-picker" }],
    Edit: [{ label: "Keyboard shortcuts", action: "shortcuts" }],
    View: [{ label: "Light appearance", action: "light" }, { label: "Dark appearance", action: "dark" }, { label: "Chat workspace", action: "workspace" }],
    Help: [{ label: "About AI Harness", action: "about" }],
  };
  return <header className={styles.desktopMenuBar}>
    <button className={styles.brand} onClick={() => onAction("model-picker")} aria-label="Open model picker" title="AI Harness Home"><img className={styles.brandLogo} src={logoUrl} alt="AI Harness" /></button>
    <div className={styles.topNavControls}>
      <button
        className={`${styles.navControlBtn} ${sidebarCollapsed ? styles.navControlActive : ""}`}
        onClick={onToggleSidebar}
        title={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
        aria-label="Toggle sidebar"
      >
        <SidebarSimple weight="light" />
      </button>
      <button
        className={styles.navControlBtn}
        onClick={() => onAction("model-picker")}
        title="Back to Model Catalog"
        aria-label="Go back"
      >
        <CaretLeft weight="light" />
      </button>
      <button
        className={styles.navControlBtn}
        onClick={() => onAction("workspace")}
        title="Forward to Chat Workspace"
        aria-label="Go forward"
      >
        <CaretRight weight="light" />
      </button>
    </div>
    <nav className={styles.menuList} aria-label="Application menu">
      {(Object.keys(menus) as MenuName[]).map((menu) => <div className={styles.menuGroup} key={menu}>
        <button className={`${styles.menuTrigger} ${activeMenu === menu ? styles.menuTriggerActive : ""}`} onClick={() => onMenuChange(activeMenu === menu ? null : menu)} aria-haspopup="menu" aria-expanded={activeMenu === menu}>{menu}</button>
        {activeMenu === menu && <div className={styles.menuPopover} role="menu" aria-label={`${menu} menu`}>
          {menus[menu].map((item) => <button key={item.label} role="menuitem" onClick={() => onAction(item.action)}>{item.label}</button>)}
        </div>}
      </div>)}
    </nav>
    <div className={styles.dragRegion} data-tauri-drag-region aria-hidden="true" onMouseDown={(event) => { if (event.button === 0) void startWindowDrag().catch((error: unknown) => reportWindowError("Window drag", error)); }} />
    <div className={styles.windowControls} aria-label="Window controls">
      <button className={styles.windowControl} onClick={() => void controlWindow("minimize").catch((error: unknown) => reportWindowError("Minimize", error))} aria-label="Minimize window"><Minus /></button>
      <button className={styles.windowControl} onClick={() => void controlWindow("maximize").catch((error: unknown) => reportWindowError("Maximize", error))} aria-label="Maximize or restore window"><Square /></button>
      <button className={`${styles.windowControl} ${styles.closeControl}`} onClick={() => void controlWindow("close").catch((error: unknown) => reportWindowError("Close", error))} aria-label="Close window"><X /></button>
    </div>
  </header>;
}

interface ModelPickerProps {
  catalogLoading: boolean;
  detailsLoading: boolean;
  models: CatalogModel[];
  selectedModel: CatalogModel | null;
  selectedFile: string;
  installed?: InstalledModel;
  download: DownloadProgress | null;
  onSearch: (query: string) => Promise<void>;
  onSelect: (model: CatalogModel) => Promise<void>;
  onFileChange: (file: string) => void;
  onInstall: () => Promise<void>;
  onOpenInstalled: () => void;
}

function ModelPicker({ catalogLoading, detailsLoading, models, selectedModel, selectedFile, installed, download, onSearch, onSelect, onFileChange, onInstall, onOpenInstalled }: ModelPickerProps) {
  const [query, setQuery] = useState("");
  const selectedDownloading = download && selectedModel && download.repoId === selectedModel.id && download.fileName === selectedFile;
  const selectedModelFile = selectedModel?.files.find((file) => file.name === selectedFile);
  return <section className={styles.picker} aria-labelledby="picker-heading">
    <div className={styles.intro}>
      <Badge size="sm">First launch</Badge>
      <h1 id="picker-heading">Choose a local model to get started.</h1>
      <p>Search the public Hugging Face GGUF catalog. AI Harness downloads your selected file to this computer, verifies its SHA-256 checksum, then unlocks chat.</p>
    </div>
    <form className={styles.searchBar} onSubmit={(event) => { event.preventDefault(); void onSearch(query); }}>
      <MagnifyingGlass aria-hidden="true" />
      <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search GGUF models, creators, or families" aria-label="Search Hugging Face GGUF models" />
      <Button type="submit" variant="secondary" size="sm" loading={catalogLoading}>Search</Button>
    </form>
    <div className={styles.modelGrid} aria-busy={catalogLoading}>
      {catalogLoading && Array.from({ length: 6 }).map((_, index) => <Card key={index} className={styles.modelCard} loading>Loading model</Card>)}
      {!catalogLoading && models.map((model) => <button className={`${styles.modelCard} ${selectedModel?.id === model.id ? styles.modelCardSelected : ""}`} key={model.id} onClick={() => void onSelect(model)} aria-pressed={selectedModel?.id === model.id}>
        <div className={styles.modelHeader}><div><span className={styles.modelFamily}>{model.author ?? "Hugging Face"}</span><h2>{model.id.split("/").at(-1)}</h2></div>{selectedModel?.id === model.id && <Badge variant="success" size="sm">Selected</Badge>}</div>
        <p className={styles.repoId}>{model.id}</p>
        <div className={styles.modelMeta}><span><strong>{model.files.length}</strong> GGUF files</span><span><strong>{model.downloads.toLocaleString()}</strong> downloads</span><span><strong>{model.likes.toLocaleString()}</strong> likes</span></div>
      </button>)}
    </div>
    {!catalogLoading && !models.length && <p className={styles.emptyCatalog}>No GGUF models matched that search. Try another phrase.</p>}
    {selectedModel && <Card className={styles.selectionSummary}>
      <Card.Body className={styles.selectionBody}>
        <div className={styles.selectionInfo}><span className={styles.summaryLabel}>Selected model file</span><h2>{selectedModel.id}</h2>
          <label className={styles.fileLabel}>GGUF file<select value={selectedFile} disabled={detailsLoading || Boolean(selectedDownloading)} onChange={(event) => onFileChange(event.target.value)}>{selectedModel.files.map((file) => <option value={file.name} key={file.name}>{file.name} — {formatBytes(file.size)}</option>)}</select></label>
          {selectedModelFile?.sha256 ? <p>SHA-256 available · {formatBytes(selectedModelFile.size)} download</p> : <p>Loading checksum metadata…</p>}
        </div>
        <div className={styles.summaryActions}>
          {installed ? <Button iconPrefix={<ChatCircleDots />} onClick={onOpenInstalled}>Open chat</Button> : <Button iconPrefix={<DownloadSimple />} onClick={() => void onInstall()} disabled={detailsLoading || !selectedModelFile?.sha256 || Boolean(download)} loading={detailsLoading}>Install model</Button>}
        </div>
      </Card.Body>
      {selectedDownloading && <div className={styles.downloadProgress}><div className={styles.progressCopy}><span>{download.stage === "verifying" ? "Verifying SHA-256 checksum…" : `Downloading ${formatBytes(download.downloadedBytes)}${download.totalBytes ? ` of ${formatBytes(download.totalBytes)}` : ""}`}</span><strong>{download.percent ?? "…"}{download.percent !== undefined ? "%" : ""}</strong></div><div className={styles.progressTrack}><span style={{ width: `${download.percent ?? 5}%` }} /></div></div>}
    </Card>}
  </section>;
}

const getSessionInitial = (title: string) => {
  const trimmed = title.trim();
  if (!trimmed) return "C";
  const firstChar = Array.from(trimmed)[0];
  return firstChar ? firstChar.toUpperCase() : "C";
};

function ChatWorkspace({ model, newChatRequest, sidebarCollapsed, onBack, onNotify }: { model: InstalledModel; newChatRequest: number; sidebarCollapsed: boolean; onBack: () => void; onNotify: (message: string) => void }) {
  const [starting, setStarting] = useState(false);
  const [engineStarted, setEngineStarted] = useState(false);
  const [engineInfo, setEngineInfo] = useState<{ backend: string; gpuLayers: number; contextSize: number; runtimeRelease: string; fallbackReason?: string } | null>(null);
  const [messages, setMessages] = useState<ConversationMessage[]>([]);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [activeSessionModelId, setActiveSessionModelId] = useState<string | undefined>();
  const [sessionQuery, setSessionQuery] = useState("");
  const [draft, setDraft] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [activeTab, setActiveTab] = useState<"home" | "code">("home");
  const [showSearch, setShowSearch] = useState(false);
  const [profileMenuOpen, setProfileMenuOpen] = useState(false);
  const [profiles, setProfiles] = useState<{ id: string; name: string; tag: string }[]>([
    { id: "default", name: "Pichyy", tag: "Local Profile" }
  ]);
  const [activeProfileId, setActiveProfileId] = useState("default");
  const activeProfile = useMemo(() => profiles.find((p) => p.id === activeProfileId) ?? profiles[0], [profiles, activeProfileId]);
  const streamAbort = useRef<AbortController | null>(null);
  const transcript = useRef<HTMLDivElement | null>(null);
  const stickToBottom = useRef(true);
  const [isAtBottom, setIsAtBottom] = useState(true);
  const pendingDelta = useRef("");
  const pendingAnimationFrame = useRef<number | null>(null);
  // A few backend safeguards can replace a completed draft (for example when
  // retrying a repetition loop, executing a native tool, or enforcing a
  // saved constraint). Keep that replacement off-screen until it is complete
  // so the answer does not visibly clear and restart two or three times.
  const replacementInProgress = useRef(false);
  const replacementContent = useRef("");
  const [promptQueue, setPromptQueue] = useState<string[]>([]);
  const [pendingChoice, setPendingChoice] = useState<PendingChoice | null>(null);
  const [isAiderMode, setIsAiderMode] = useState(true);
  const [workspacePath, setWorkspacePath] = useState("c:\\Users\\Newsk\\Downloads\\Aphelion");

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let currentMode: "thinking" | "answer" | "none" = "none";

    const decodeUnicodeStr = (str: string) => {
      try {
        return str.replace(/\\u([0-9a-fA-F]{4})/g, (_, code) =>
          String.fromCharCode(parseInt(code, 16))
        );
      } catch {
        return str;
      }
    };

    void listen<{ session_id: string; event_type: string; content: string }>("aider-event", (event) => {
      const { event_type, content: rawContent } = event.payload;
      if (event_type === "stdout" || event_type === "stderr" || event_type === "error") {
        const cleanLine = decodeUnicodeStr(rawContent).trim();

        // Skip CLI noise, header lines & leaked JSON payloads
        if (
          !cleanLine ||
          cleanLine.startsWith("{") ||
          cleanLine.includes("Can't initialize prompt toolkit") ||
          cleanLine.includes("Terminal does not support") ||
          cleanLine.includes("Warning for") ||
          cleanLine.includes("https://aider.chat") ||
          cleanLine.includes("Scanning repo") ||
          cleanLine.includes("Aider v") ||
          cleanLine.includes("Model:") ||
          cleanLine.includes("Git repo:") ||
          cleanLine.includes("Repo-map:") ||
          cleanLine.includes("Initial repo scan") ||
          cleanLine.includes("Has it been deleted") ||
          cleanLine.includes("Cur working dir:") ||
          cleanLine.includes("Git working dir:") ||
          cleanLine.includes("Note: in-chat filenames") ||
          cleanLine.includes("Summarization failed") ||
          cleanLine.includes("summarizer unexpectedly") ||
          cleanLine.includes("Process exited with status") ||
          cleanLine.includes("Commit ") ||
          cleanLine.includes("Applied edit") ||
          cleanLine.includes("ไม่มีคำสั่ง shell") ||
          cleanLine.includes("You can skip this check") ||
          cleanLine.includes("Added .aider*") ||
          cleanLine.startsWith("---") ||
          cleanLine.startsWith("===") ||
          cleanLine.startsWith("Tokens:") ||
          cleanLine.startsWith("───")
        ) {
          return;
        }

        if (cleanLine === "► THINKING" || cleanLine === "THINKING" || cleanLine.startsWith("► THINKING")) {
          currentMode = "thinking";
          return;
        }

        if (cleanLine === "► ANSWER" || cleanLine === "ANSWER" || cleanLine.startsWith("► ANSWER")) {
          currentMode = "answer";
          return;
        }

        // Clean off any leading ► THINKING or ► ANSWER prefixes if mixed on the line
        const textLine = cleanLine.replace(/^►\s*(THINKING|ANSWER)\s*/i, "").trim();
        if (!textLine) return;

        const isThinking = currentMode === "thinking";
        const isAnswer = currentMode === "answer" || currentMode === "none";

        setMessages((current) => current.map((msg, idx) => {
          if (idx !== current.length - 1 || msg.role !== "assistant") return msg;
          const processArr = (msg.process ?? []).filter(p => p !== "⚡ Launching embedded Aider engine...");

          if (isThinking) {
            return {
              ...msg,
              process: [...processArr, textLine],
            };
          } else if (isAnswer) {
            const nextContent = msg.content ? `${msg.content}\n${textLine}` : textLine;
            return {
              ...msg,
              process: processArr,
              content: nextContent,
            };
          }
          return msg;
        }));
      } else if (event_type === "done") {
        setStreaming(false);
        currentMode = "none";
      }
    }).then((h) => { unlisten = h; });
    return () => unlisten?.();
  }, []);

  useEffect(() => {
    let unlistenFn: (() => void) | undefined;
    let isCancelled = false;
    void listen<PendingChoice>("ai-interaction-request", (event) => {
      setPendingChoice(event.payload);
    }).then((cleanup) => {
      if (isCancelled) {
        cleanup();
      } else {
        unlistenFn = cleanup;
      }
    });
    return () => {
      isCancelled = true;
      unlistenFn?.();
    };
  }, []);
  const promptQueueRef = useRef<string[]>([]);

  const refreshSessions = useCallback(async (query = sessionQuery) => {
    try {
      setSessions(await invoke<SessionSummary[]>("list_sessions", { query: query || undefined }));
    } catch (error) {
      onNotify(`Could not load saved chats: ${String(error)}`);
    }
  }, [onNotify, sessionQuery]);

  const openSession = useCallback(async (sessionId: string) => {
    if (streaming) return;
    if (activeSessionId && activeSessionId !== sessionId) {
      void invoke("trigger_session_end_memory", { sessionId: activeSessionId }).catch(() => {});
    }
    try {
      const detail = await invoke<SessionDetail>("get_session", { sessionId });
      setActiveSessionId(detail.session.id);
      setActiveSessionModelId(detail.session.modelId);
      setMessages(detail.messages.map((message) => ({ role: message.role, content: message.content, process: message.thinkingSummary ? [message.thinkingSummary] : [], sources: message.webSources, retrievalTrace: message.retrievalTrace })));
      if (detail.session.modelId && detail.session.modelId !== model.repoId) onNotify(`This chat was created with ${detail.session.modelId}. Select that model before continuing.`);
    } catch (error) {
      onNotify(`Could not open saved chat: ${String(error)}`);
    }
  }, [activeSessionId, model.repoId, onNotify, streaming]);

  const flushPendingDelta = useCallback(() => {
    pendingAnimationFrame.current = null;
    const delta = pendingDelta.current;
    pendingDelta.current = "";
    if (!delta) return;
    setMessages((current) => current.map((message, index) => index === current.length - 1 ? { ...message, content: message.content + delta } : message));
  }, []);

  const commitGenerationContent = useCallback((content: string) => {
    setMessages((current) => current.map((message, index) => index === current.length - 1 && message.role === "assistant"
      ? { ...message, content }
      : message));
    replacementInProgress.current = false;
    replacementContent.current = "";
  }, []);

  useEffect(() => () => {
    streamAbort.current?.abort();
    if (pendingAnimationFrame.current !== null) cancelAnimationFrame(pendingAnimationFrame.current);
  }, []);
  useEffect(() => {
    const timer = window.setTimeout(() => { void refreshSessions(); }, 200);
    return () => window.clearTimeout(timer);
  }, [refreshSessions, sessionQuery]);
  useEffect(() => {
    if (streaming) return;
    if (activeSessionId) {
      void invoke("trigger_session_end_memory", { sessionId: activeSessionId }).catch(() => {});
    }
    setActiveSessionId(null);
    setActiveSessionModelId(undefined);
    setMessages([]);
  }, [newChatRequest]);
  const updateScrollState = useCallback(() => {
    const element = transcript.current;
    if (!element) return;
    const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
    stickToBottom.current = atBottom;
    setIsAtBottom(atBottom);
  }, []);

  const scrollToLatest = useCallback((behavior: ScrollBehavior = "smooth") => {
    const element = transcript.current;
    if (!element) return;
    stickToBottom.current = true;
    setIsAtBottom(true);
    element.scrollTo({ top: element.scrollHeight, behavior });
  }, []);

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const element = transcript.current;
      if (!element) return;
      // During generation the transcript is deliberately manual-scroll: new
      // tokens never move the viewport. The down-arrow becomes the explicit
      // way to catch up with the latest output.
      if (streaming) {
        updateScrollState();
      } else if (stickToBottom.current) {
        element.scrollTop = element.scrollHeight;
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [messages, streaming, updateScrollState]);
  const startEngine = async () => {
    setStarting(true);
    try {
      const info = await invoke<{ backend: string; gpuLayers: number; contextSize: number; runtimeRelease: string; fallbackReason?: string }>("start_engine", { modelFile: model.localFile });
      setEngineInfo(info);
      setEngineStarted(true);
      onNotify(info.fallbackReason ?? `Local ${info.backend.toUpperCase()} engine is ready (${info.gpuLayers === -1 ? "maximum safe GPU offload" : `${info.gpuLayers} GPU layers`}, ${info.contextSize}-token context).`);
    } catch (error) {
      onNotify(`Could not start the local engine: ${String(error)}`);
    } finally {
      setStarting(false);
    }
  };

  const createSessionForMessage = async () => {
    if (activeSessionId) return { id: activeSessionId, isNew: false };
    const session = await invoke<SessionSummary>("create_session", { modelId: model.repoId });
    setActiveSessionId(session.id);
    setActiveSessionModelId(session.modelId);
    return { id: session.id, isNew: true };
  };

  const renameSession = async (session: SessionSummary) => {
    const title = window.prompt("Rename chat", session.title);
    if (!title?.trim()) return;
    try {
      await invoke("rename_session", { sessionId: session.id, title });
      await refreshSessions();
    } catch (error) { onNotify(`Could not rename chat: ${String(error)}`); }
  };

  const deleteSession = async (session: SessionSummary) => {
    if (!window.confirm(`Delete "${session.title}"? This cannot be undone.`)) return;
    try {
      await invoke("delete_session", { sessionId: session.id });
      if (session.id === activeSessionId) {
        setActiveSessionId(null);
        setActiveSessionModelId(undefined);
        setMessages([]);
      }
      await refreshSessions();
    } catch (error) { onNotify(`Could not delete chat: ${String(error)}`); }
  };

  const startNewChat = () => {
    if (streaming) return;
    if (activeSessionId) {
      void invoke("trigger_session_end_memory", { sessionId: activeSessionId })
        .catch((error) => onNotify(`Could not finalize chat memory: ${String(error)}`));
    }
    setActiveSessionId(null);
    setActiveSessionModelId(undefined);
    setMessages([]);
    setDraft("");
    setWorkspacePath("");
  };

  const sendMessage = async (
    overrideContent?: string,
    interaction?: { id: string; optionId: string },
  ) => {
    const content = (overrideContent ?? draft).trim();
    if (!content || !engineStarted) return;
    if (streaming) {
      promptQueueRef.current = [...promptQueueRef.current, content];
      setPromptQueue([...promptQueueRef.current]);
      setDraft("");
      setMessages((current) => [
        ...current,
        { role: "user", content, isQueued: true },
      ]);
      onNotify(`Prompt queued (#${promptQueueRef.current.length} in line)`);
      return;
    }

    if (activeSessionModelId && activeSessionModelId !== model.repoId) {
      onNotify(`This saved chat requires ${activeSessionModelId}. Change model before continuing.`);
      return;
    }
    let session: { id: string; isNew: boolean };
    try {
      session = await createSessionForMessage();
    } catch (error) {
      onNotify(`Could not create saved chat: ${String(error)}`);
      return;
    }
    const userMessage: ConversationMessage = { role: "user", content, isQueued: false };
    setDraft("");
    setMessages((current) => {
      const unqueued = current.map((m) => (m.content === content && m.isQueued ? { ...m, isQueued: false } : m));
      const activeMsgs = unqueued.filter((m) => !m.isQueued);
      if (!activeMsgs.some((m) => m.role === "user" && m.content === content)) {
        activeMsgs.push(userMessage);
      }
      const initialProcess = isAiderMode
        ? ["⚡ Launching embedded Aider engine..."]
        : ["Starting request and checking what needs live information"];
      return [...activeMsgs, { role: "assistant", content: "", process: initialProcess, retrievalTrace: [] }];
    });

    const requestMessages = [...messages.filter((m) => !m.isQueued), userMessage];
    setStreaming(true);

    if (isAiderMode) {
      try {
        const result = await invoke<string>("run_aider_coding_task", {
          sessionId: session.id,
          workspacePath: workspacePath || "c:\\Users\\Newsk\\Downloads\\Aphelion",
          prompt: content,
          autoCommits: true,
        });
        setMessages((current) => current.map((msg, idx) => idx === current.length - 1 && msg.role === "assistant"
          ? { ...msg, content: result || "Aider coding task completed." }
          : msg));
      } catch (error) {
        const errStr = String(error);
        setToast(`Aider error: ${errStr}`);
        setMessages((current) => current.map((msg, idx) => idx === current.length - 1 && msg.role === "assistant"
          ? { ...msg, process: (msg.process ?? []).concat(`❌ ${errStr}`) }
          : msg));
      } finally {
        setStreaming(false);
      }
      return;
    }

    pendingDelta.current = "";
    replacementInProgress.current = false;
    replacementContent.current = "";
    const controller = new AbortController();
    streamAbort.current = controller;
    try {
      const result = await streamLocalChat({
        messages: requestMessages,
        sessionId: session.id,
        interactionId: interaction?.id,
        interactionOptionId: interaction?.optionId,
        signal: controller.signal,
        onDelta: (delta) => {
          if (replacementInProgress.current) {
            replacementContent.current += delta;
            return;
          }
          pendingDelta.current += delta;
          if (pendingAnimationFrame.current === null) pendingAnimationFrame.current = requestAnimationFrame(flushPendingDelta);
        },
        onTrim: (suffix) => {
          flushPendingDelta();
          if (suffix) {
            replacementInProgress.current = true;
            replacementContent.current = "";
          }
        },
        onStatus: (status) => {
          setMessages((current) => current.map((message, index) => index === current.length - 1 && message.role === "assistant"
            ? { ...message, process: message.process?.at(-1) === status ? message.process : [...(message.process ?? []), status] }
            : message));
        },
        onRetrievalTrace: (entry) => {
          setMessages((current) => current.map((message, index) => index === current.length - 1 && message.role === "assistant"
            ? { ...message, retrievalTrace: [...(message.retrievalTrace ?? []), entry] }
            : message));
        },
      });
      flushPendingDelta();
      if (result) {
        commitGenerationContent(result.content);
      }
      if (result?.sources.length) {
        setMessages((current) => current.map((message, index) => index === current.length - 1 && message.role === "assistant"
          ? { ...message, sources: result.sources }
          : message));
      }
      if (result?.retrievalTrace.length) {
        setMessages((current) => current.map((message, index) => index === current.length - 1 && message.role === "assistant"
          ? { ...message, retrievalTrace: result.retrievalTrace }
          : message));
      }
      await refreshSessions();
      if (session.isNew && result?.content.trim()) {
        void invoke("generate_session_title", { sessionId: session.id }).then(() => refreshSessions()).catch((error) => onNotify(`Could not title chat: ${String(error)}`));
      }
    } catch (error) {
      if (replacementInProgress.current && replacementContent.current) {
        commitGenerationContent(replacementContent.current);
      }
      if (!controller.signal.aborted) {
        setMessages((current) => current.map((message, index) => index === current.length - 1 && !message.content ? { ...message, content: "Sorry, the local engine could not complete that response." } : message));
        onNotify(`Message failed: ${String(error)}`);
      }
    } finally {
      setStreaming(false);
      if (pendingAnimationFrame.current !== null) cancelAnimationFrame(pendingAnimationFrame.current);
      flushPendingDelta();
      if (replacementInProgress.current && replacementContent.current) {
        commitGenerationContent(replacementContent.current);
      }
      if (streamAbort.current === controller) streamAbort.current = null;

      if (promptQueueRef.current.length > 0) {
        const nextPrompt = promptQueueRef.current.shift()!;
        setPromptQueue([...promptQueueRef.current]);
        setTimeout(() => {
          void sendMessage(nextPrompt);
        }, 100);
      }
    }
  };

  const usedChars = messages.reduce((acc, m) => acc + m.content.length, 0) + draft.length;
  const maxContextTokens = engineInfo?.contextSize ?? 8192;

  return <section className={`${styles.workspace} ${sidebarCollapsed ? styles.workspaceCollapsed : ""}`} aria-label="Chat workspace">
    <aside className={`${styles.sidebar} ${sidebarCollapsed ? styles.sidebarCollapsed : ""}`} aria-label="Sidebar navigation">
      <div className={styles.sidebarHeader}>
        <nav className={styles.topPillNav} aria-label="Navigation modes">
          <button
            type="button"
            className={`${styles.tabPill} ${activeTab === "home" ? styles.tabPillActive : ""}`}
            onClick={() => setActiveTab("home")}
            title="Home"
          >
            <House weight="light" />
            <span>Home</span>
          </button>
          <button
            type="button"
            className={`${styles.tabPill} ${activeTab === "code" ? styles.tabPillActive : ""}`}
            onClick={() => {
              setActiveTab("code");
              onNotify("Code mode coming soon!");
            }}
            title="Code (Coming soon)"
          >
            <Code weight="light" />
            <span>Code</span>
          </button>
        </nav>
      </div>

      <div className={styles.sidebarContent}>
        <Button
          fullWidth
          size="sm"
          iconPrefix={<Plus weight="light" />}
          onClick={startNewChat}
          disabled={streaming}
          title="New chat"
        >
          {!sidebarCollapsed && "New chat"}
        </Button>

        <div className={styles.sidebarNavGroup}>
          <button
            type="button"
            className={styles.sidebarNavItem}
            onClick={() => onNotify("Projects feature coming soon")}
            title="Projects"
          >
            <FolderSimple weight="light" />
            <span>Projects</span>
          </button>
          <button
            type="button"
            className={styles.sidebarNavItem}
            onClick={() => onNotify("Artifacts feature coming soon")}
            title="Artifacts"
          >
            <Palette weight="light" />
            <span>Artifacts</span>
          </button>
          <button
            type="button"
            className={styles.sidebarNavItem}
            onClick={onBack}
            title="Customize / Change model"
          >
            <Gear weight="light" />
            <span>Customize</span>
          </button>
        </div>

        <section className={styles.sessionBrowser} aria-label="Saved chats">
          <div className={styles.recentsHeader}>
            <span className={styles.sidebarEyebrow}>Recents</span>
            <button
              type="button"
              className={`${styles.filterToggleBtn} ${showSearch ? styles.filterToggleBtnActive : ""}`}
              onClick={() => setShowSearch((prev) => !prev)}
              title="Search & Filter Recents"
              aria-label="Filter recents"
            >
              <MagnifyingGlass weight="bold" />
            </button>
          </div>

          {showSearch && (
            <div className={styles.sessionSearch}>
              <MagnifyingGlass aria-hidden="true" />
              <input
                value={sessionQuery}
                onChange={(event) => setSessionQuery(event.target.value)}
                placeholder="Search chats..."
                aria-label="Search saved chats"
              />
            </div>
          )}

          <div className={styles.sessionList}>
            {!sessions.length && (
              <p className={styles.emptySessions}>
                {sessionQuery ? "No chats found" : "Saved chats will appear here"}
              </p>
            )}
            {sessions.map((session) => (
              <div
                className={`${styles.sessionItem} ${activeSessionId === session.id ? styles.sessionItemActive : ""}`}
                key={session.id}
                title={sidebarCollapsed ? session.title : undefined}
              >
                <div className={styles.sessionAvatar} title={session.title}>
                  {getSessionInitial(session.title)}
                </div>
                <button
                  type="button"
                  className={styles.sessionOpen}
                  onClick={() => void openSession(session.id)}
                  disabled={streaming}
                  aria-current={activeSessionId === session.id ? "page" : undefined}
                  title={session.title}
                >
                  <span>{session.title}</span>
                  <small>{formatSessionTime(session.updatedAt)}</small>
                </button>
                <div className={styles.sessionActions}>
                  <button type="button" onClick={() => void renameSession(session)} aria-label={`Rename ${session.title}`}>
                    <PencilSimple />
                  </button>
                  <button type="button" onClick={() => void deleteSession(session)} aria-label={`Delete ${session.title}`}>
                    <Trash />
                  </button>
                </div>
              </div>
            ))}
          </div>
        </section>
      </div>

      <div className={styles.sidebarFooter}>
        <div className={styles.profileRow}>
          <button
            type="button"
            className={styles.profileTrigger}
            onClick={() => setProfileMenuOpen((prev) => !prev)}
            title={`${activeProfile.name} (${activeProfile.tag})`}
            aria-expanded={profileMenuOpen}
          >
            <div className={styles.profileInfo}>
              <div className={styles.profileAvatar}>{activeProfile.name.charAt(0).toUpperCase()}</div>
              <div className={styles.profileText}>
                <span className={styles.profileName}>{activeProfile.name}</span>
                <span className={styles.profileTag}>{activeProfile.tag}</span>
              </div>
            </div>
            {profileMenuOpen ? <CaretUp weight="bold" /> : <CaretDown weight="bold" />}
          </button>

          {profileMenuOpen && (
            <div className={styles.profileMenu}>
              <button
                type="button"
                className={styles.profileMenuItem}
                onClick={() => {
                  setProfileMenuOpen(false);
                  const name = prompt("Enter new local profile name:");
                  if (name && name.trim()) {
                    const newP = { id: String(Date.now()), name: name.trim(), tag: "Local Profile" };
                    setProfiles((prev) => [...prev, newP]);
                    setActiveProfileId(newP.id);
                    onNotify(`Switched to new profile: ${newP.name}`);
                  }
                }}
              >
                <UserPlus weight="bold" />
                <span>Create Local Profile</span>
              </button>
              {profiles.map((p) => (
                <button
                  type="button"
                  key={p.id}
                  className={styles.profileMenuItem}
                  onClick={() => {
                    setActiveProfileId(p.id);
                    setProfileMenuOpen(false);
                    onNotify(`Active profile: ${p.name}`);
                  }}
                >
                  <User weight={p.id === activeProfileId ? "fill" : "regular"} />
                  <span>{p.name} {p.id === activeProfileId ? "(Active)" : ""}</span>
                </button>
              ))}
              <hr style={{ border: 0, borderTop: "1px solid var(--border)", margin: "4px 0" }} />
              <button
                type="button"
                className={styles.profileMenuItem}
                onClick={() => {
                  setProfileMenuOpen(false);
                  onBack();
                }}
              >
                <Gear weight="bold" />
                <span>Change Model / Settings</span>
              </button>
            </div>
          )}
        </div>
      </div>
    </aside>
    <div className={styles.chatPanel}>
      <div style={{ padding: "8px 16px", borderBottom: "1px solid rgba(255, 255, 255, 0.06)", display: "flex", justifyContent: "flex-end" }}>
        <WorkspaceFolderPicker
          currentWorkspace={workspacePath}
          onSelectWorkspace={setWorkspacePath}
          isAiderMode={isAiderMode}
          onToggleAiderMode={setIsAiderMode}
        />
      </div>
      <div className={styles.transcriptRegion}>
      <div ref={transcript} className={styles.chatTranscript} onScroll={updateScrollState} aria-live="polite" aria-busy={streaming}>
        {!messages.length && <div className={styles.chatEmpty}><img className={styles.emptyLogo} src={logoUrl} alt="" /><h1>Your local model is ready.</h1><p>{engineStarted ? "Ask anything. Responses stay on this computer." : `${model.fileName} is verified and ready. Start the bundled engine to begin a conversation.`}</p><Button iconPrefix={<Play />} onClick={() => void startEngine()} loading={starting} disabled={engineStarted}>{engineStarted ? "Engine running" : "Start local engine"}</Button></div>}
        {messages.map((message, index) => {
          const isStreamingMessage = streaming && message.role === "assistant" && index === messages.length - 1;
          return <article className={`${styles.message} ${message.role === "user" ? styles.userMessage : styles.assistantMessage}`} key={`${message.role}-${index}`}>
            {message.role === "assistant" && (
              <ThinkingSummary process={message.process ?? []} retrievalTrace={message.retrievalTrace ?? []} sources={message.sources ?? []} streaming={isStreamingMessage} />
            )}
            {message.role === "assistant" ? (
              <MarkdownMessage content={message.content} sources={message.sources ?? []} streaming={isStreamingMessage} />
            ) : (
              <div className={styles.userMessageContent}>
                <p>{message.content}</p>
                {message.isQueued && <span className={styles.queuedBadge}>⏳ Queued</span>}
              </div>
            )}
          </article>;
        })}
      </div>
      {!isAtBottom && messages.length > 0 && <Button type="button" variant="secondary" className={styles.scrollToLatest} aria-label="Scroll to latest message" onClick={() => scrollToLatest()}><ArrowDown weight="bold" /></Button>}
      </div>
        {pendingChoice && (
          <InteractiveChoiceBox
            question={pendingChoice.question}
            options={pendingChoice.options}
            disabled={streaming}
            onSubmit={(optionId, answer) => {
              const selectedOptionId = optionId ?? answer;
              setPendingChoice(null);
              setDraft("");
              void sendMessage(answer, { id: pendingChoice.id, optionId: selectedOptionId });
            }}
            onDismiss={() => setPendingChoice(null)}
          />
        )}
      <form className={styles.composer} onSubmit={(event) => { event.preventDefault(); if (workspacePath) void sendMessage(); }}>
        <Textarea
          aria-label="Message the model"
          placeholder={!workspacePath ? "กรุณาเลือกโฟลเดอร์โครงการ (Workspace Folder) ด้านบนเพื่อเริ่มคุยกับ Agent..." : engineStarted ? (streaming ? "Type next prompt to queue..." : "Message AI Harness") : "Start the local engine to begin chatting"}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey && workspacePath) { event.preventDefault(); void sendMessage(); } }}
          disabled={!engineStarted || !workspacePath}
          rows={2}
        />
        <div className={styles.composerFooter}>
          <div className={styles.composerMeta}>
            <Plus aria-hidden="true" />
            <span>{!workspacePath ? "Workspace Required" : streaming ? "Generating" : engineStarted ? "Local engine" : "Engine offline"}</span>
            {promptQueue.length > 0 && <span className={styles.queueCountBadge}>{promptQueue.length} queued</span>}
          </div>
          <div className={styles.composerRightActions}>
            <ContextDonutChart usedChars={usedChars} maxTokens={maxContextTokens} />
            {streaming ? (
              <div className={styles.streamingControlGroup}>
                {draft.trim() && (
                  <Button type="submit" size="sm" iconPrefix={<Plus weight="bold" />} disabled={!workspacePath}>
                    Queue ({promptQueue.length + 1})
                  </Button>
                )}
                <Button
                  type="button"
                  variant="secondary"
                  className={styles.composerRoundAction}
                  iconPrefix={<Stop weight="fill" />}
                  onClick={() => {
                    promptQueueRef.current = [];
                    setPromptQueue([]);
                    streamAbort.current?.abort();
                  }}
                  title="Stop generating"
                />
              </div>
            ) : (
              <Button type="submit" variant="primary" className={styles.composerRoundAction} iconPrefix={<ArrowUp weight="bold" />} disabled={!engineStarted || !draft.trim() || !workspacePath} title="Send message" />
            )}
          </div>
        </div>
      </form>
    </div>
  </section>;
}

function ContextDonutChart({ usedChars = 0, maxTokens = 8192 }: { usedChars?: number; maxTokens?: number }) {
  const safeUsedChars = Math.max(0, usedChars || 0);
  const safeMaxTokens = Math.max(1, maxTokens || 8192);
  const estimatedTokens = Math.ceil(safeUsedChars / 3.8);
  const percentage = Math.min(100, Math.round((estimatedTokens / safeMaxTokens) * 100));

  const radius = 9;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset = circumference - (percentage / 100) * circumference;

  const color =
    percentage >= 85
      ? "#ef4444"
      : percentage >= 65
      ? "#f59e0b"
      : "var(--accent, #10b981)";

  return (
    <div
      className={styles.contextDonutWrapper}
      tabIndex={0}
      role="region"
      aria-label={`Context usage: ${percentage}%`}
    >
      <svg className={styles.contextDonutSvg} viewBox="0 0 24 24">
        <circle className={styles.contextDonutBg} cx="12" cy="12" r={radius} />
        <circle
          className={styles.contextDonutMeter}
          cx="12"
          cy="12"
          r={radius}
          style={{
            stroke: color,
            strokeDasharray: circumference,
            strokeDashoffset: strokeDashoffset,
          }}
        />
      </svg>
      <div className={styles.contextTooltip}>
        <div className={styles.contextTooltipHeader}>
          <span>Context Window</span>
          <span>{percentage}%</span>
        </div>
        <div className={styles.contextTooltipValue}>
          {estimatedTokens.toLocaleString()} / {safeMaxTokens.toLocaleString()} tokens
        </div>
        <div className={styles.contextTooltipSub}>
          ~{safeUsedChars.toLocaleString()} characters used
        </div>
      </div>
    </div>
  );
}

const openExternalUrl = async (url?: string) => {
  if (!url) return;
  try {
    await openShellUrl(url);
  } catch (err) {
    console.warn("openShellUrl failed, fallback:", err);
    try {
      await invoke("plugin:shell|open", { path: url });
    } catch {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  }
};

function extractDomain(url?: string): string {
  if (!url) return "web";
  try {
    const host = new URL(url).hostname;
    return host.replace(/^www\./, "");
  } catch {
    return "web";
  }
}

function GlobeIcon({ domain }: { domain: string }) {
  return (
    <div className={styles.domainFavicon}>
      <span>{domain.charAt(0).toUpperCase()}</span>
    </div>
  );
}

function preprocessLatex(content: string) {
  if (!content) return content;
  return content
    .split(/(```[\s\S]*?```|`[^`]+`)/g)
    .map((part) => {
      if (part.startsWith("```") || part.startsWith("`")) return part;
      return part
        .replace(/\\\[([\s\S]*?)\\\]/g, (_, math) => `$$${math}$$`)
        .replace(/\\\(([\s\S]*?)\\\)/g, (_, math) => `$${math}$`);
    })
    .join("");
}

function MarkdownMessage({ content, sources, streaming }: { content: string; sources: WebSource[]; streaming: boolean }) {
  const cleanedContent = content;
  if (!cleanedContent) return <p>{streaming ? <MorphingInfinity label="Thinking…" /> : ""}</p>;
  return (
    <div className={styles.markdown}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeHighlight, rehypeKatex]}
        components={{ pre: CodeBlock, a: CitationLink(sources) }}
      >
        {preprocessLatex(citationMarkdown(cleanedContent, sources))}
      </ReactMarkdown>
      {!!sources.length && (
        <div className={styles.webSources} aria-label="Web sources">
          {sources.map((source) => (
            <a
              href={source.url}
              className={styles.citationCardBadgeItem}
              onClick={(e) => {
                e.preventDefault();
                void openExternalUrl(source.url);
              }}
              key={source.id}
              title={source.title}
            >
              <span className={styles.webSourceIndex}>[{source.id}]</span>
              <span className={styles.webSourceTitle}>{source.title}</span>
            </a>
          ))}
        </div>
      )}
      {streaming && <span className={styles.streamingCursor} aria-label="Generating" />}
    </div>
  );
}

function citationMarkdown(content: string, sources: WebSource[]) {
  if (!sources.length) return content;
  const byId = new Map(sources.map((source) => [source.id, source]));
  return content
    .split(/(```[\s\S]*?```|`[^`]+`)/g)
    .map((part) => {
      if (part.startsWith("```") || part.startsWith("`")) return part;
      return part.replace(/\[(\d+)\]/g, (citation, rawId) => {
        const source = byId.get(Number(rawId));
        return source ? `[${rawId}](<${source.url}>)` : citation;
      });
    })
    .join("");
}

function CitationLink(sources: WebSource[]) {
  return ({ href, children }: ComponentPropsWithoutRef<"a">) => {
    const source = sources.find((item) => item.url === href);
    const targetUrl = href || source?.url;
    return (
      <a
        className={source ? styles.citationBadge : styles.markdownLink}
        href={targetUrl}
        target="_blank"
        rel="noopener noreferrer"
        onClick={(e) => {
          e.preventDefault();
          if (targetUrl) void openExternalUrl(targetUrl);
        }}
        title={source?.title || href}
      >
        {children}
      </a>
    );
  };
}

function ThinkingSummary({ process, retrievalTrace, sources, streaming }: { process: string[]; retrievalTrace: RetrievalTraceEntry[]; sources: WebSource[]; streaming: boolean }) {
  if (!process.length && !retrievalTrace.length && !sources.length && !streaming) return null;

  const [toolResultsOpen, setToolResultsOpen] = useState(false);
  const [retrievalOpen, setRetrievalOpen] = useState(false);

  const hasToolCalls = process.some((step) => step.toLowerCase().includes("harness tool") || step.toLowerCase().includes("executing"));
  const commandSteps = process.filter((step) => !step.toLowerCase().includes("writing response"));

  return (
    <div className={styles.thinkingContainer}>
      {/* Tool Execution Tree (Image 2 style) */}
      {process.length > 0 && (
        <div className={styles.toolTraceBox}>
          <div className={styles.toolTraceHeader} onClick={() => setToolResultsOpen((prev) => !prev)}>
            <div className={styles.toolTraceTitle}>
              <Code size={16} weight="bold" />
              <span>{hasToolCalls ? "Ran a tool command, read workspace/history" : "Ran reasoning steps"}</span>
            </div>
            <div className={styles.toolTraceMeta}>
              <span className={styles.stepBadge}>{commandSteps.length} steps</span>
              {toolResultsOpen ? <CaretUp size={14} weight="bold" /> : <CaretDown size={14} weight="bold" />}
            </div>
          </div>

          {toolResultsOpen && (
            <div className={styles.toolTraceTree}>
              {commandSteps.map((step, idx) => (
                <div key={idx} className={styles.toolTreeNode}>
                  <span className={styles.treeConnector}>├─</span>
                  <div className={styles.treeNodeContent}>
                    {step.toLowerCase().includes("harness tool") ? (
                      <span className={styles.cmdPrefix}>&gt;_ {step}</span>
                    ) : (
                      <span className={styles.filePrefix}>📄 {step}</span>
                    )}
                  </div>
                </div>
              ))}
              <div className={styles.toolTreeNode}>
                <span className={styles.treeConnector}>└─</span>
                <div className={styles.treeDoneBadge}>
                  <Check size={14} weight="bold" />
                  <span>{streaming ? <TextShimmerWave text="Running..." /> : "Done"}</span>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Web Search Citation Card (Image 1 style) */}
      {(retrievalTrace.length > 0 || sources.length > 0) && (
        <div className={styles.searchCitationBox}>
          <div className={styles.searchCitationHeader} onClick={() => setRetrievalOpen((prev) => !prev)}>
            <div className={styles.searchCitationTitle}>
              <MagnifyingGlass size={16} weight="bold" />
              <span>Live Retrieval & Citations</span>
            </div>
            <div className={styles.searchCitationMeta}>
              <span className={styles.resultsBadge}>{sources.length || retrievalTrace.length} results</span>
              {retrievalOpen ? <CaretUp size={14} weight="bold" /> : <CaretDown size={14} weight="bold" />}
            </div>
          </div>

          {retrievalOpen && (
            <div className={styles.searchCitationBody}>
              <div className={styles.citationCardList}>
                {(sources.length > 0 ? sources : retrievalTrace).map((item: any, index) => {
                  const url = item.url || item.endpoint;
                  const domain = extractDomain(url);
                  const title = item.title || item.stage || "Source Result";
                  return (
                    <div
                      key={index}
                      className={styles.citationCardItem}
                      onClick={() => url && void openExternalUrl(url)}
                    >
                      <div className={styles.citationItemLeft}>
                        <GlobeIcon domain={domain} />
                        <div className={styles.citationItemInfo}>
                          <span className={styles.citationItemTitle}>{title}</span>
                          <span className={styles.citationItemDomain}>{domain}</span>
                        </div>
                      </div>
                      {typeof item.score === "number" && (
                        <span className={styles.citationItemScore}>{(item.score * 100).toFixed(0)}% match</span>
                      )}
                    </div>
                  );
                })}
              </div>

              <div className={styles.searchDoneFooter}>
                <Check size={14} weight="bold" />
                <span>Done</span>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}



function CodeBlock({ children }: ComponentPropsWithoutRef<"pre">) {
  const [copied, setCopied] = useState(false);
  const code = Children.toArray(children).find(isValidElement<{ className?: string; children?: ReactNode }>);
  const rawCode = textContent(code?.props.children).replace(/\n$/, "");
  const language = code?.props.className?.match(/language-([\w+-]+)/)?.[1] ?? "text";

  const copyCode = async () => {
    try {
      await navigator.clipboard.writeText(rawCode);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_600);
    } catch {
      setCopied(false);
    }
  };

  return <div className={styles.codeBlock}>
    <div className={styles.codeToolbar}><span>{language}</span><button type="button" onClick={() => void copyCode()} aria-label={`Copy ${language} code`}>{copied ? <><Check weight="bold" />Copied</> : <><Copy weight="bold" />Copy</>}</button></div>
    <pre>{children}</pre>
  </div>;
}

function textContent(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textContent).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textContent(node.props.children);
  return "";
}
