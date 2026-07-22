export interface ModelFile {
  name: string;
  size?: number;
  sha256?: string;
}

export interface CatalogModel {
  id: string;
  author?: string;
  downloads: number;
  likes: number;
  lastModified?: string;
  files: ModelFile[];
}

export interface InstalledModel {
  repoId: string;
  fileName: string;
  localFile: string;
  size: number;
  sha256: string;
  installedAt: string;
}

export interface DownloadProgress {
  repoId: string;
  fileName: string;
  stage: "downloading" | "verifying";
  downloadedBytes: number;
  totalBytes?: number;
  percent?: number;
}

export type BackendPreference = "auto" | "cpu" | "cuda" | "vulkan" | "sycl";

export interface EngineSettings {
  backend: BackendPreference;
  gpuLayers: number;
}

export interface HardwareProfile {
  gpus: string[];
  recommendedBackend: BackendPreference;
  recommendationReason: string;
  vramTotalMib?: number;
  vramUsedMib?: number;
}

export interface SessionSummary {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  modelId?: string;
}

export interface SessionMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  thinkingSummary?: string;
  finishReason?: string;
  webSources?: WebSource[];
  createdAt: string;
  sequence: number;
}

export interface SessionDetail {
  session: SessionSummary;
  messages: SessionMessage[];
  conversationMemory: string;
}

export interface WebSource {
  id: number;
  title: string;
  url: string;
}
