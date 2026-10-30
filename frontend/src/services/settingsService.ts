import { invoke } from '@tauri-apps/api/core';

export type AiProvider = 'builtin-ai' | 'ollama' | 'openai' | 'claude' | 'groq' | 'openrouter' | 'gemini' | 'deepseek' | 'custom-openai' | 'claude-code';

/** What embeds passages for search: the built-in model, an Ollama server, or a provider's API. */
export type EmbeddingProvider = 'builtin' | 'ollama' | 'openai' | 'gemini' | 'custom-openai';

export interface AiSettings {
  provider: AiProvider;
  /** Empty with builtin-ai: the best built-in model downloaded. */
  model: string;
  ollamaEndpoint: string | null;
  /** Base URL of an OpenAI-compatible server (provider custom-openai). */
  customEndpoint: string | null;
  embeddingProvider: EmbeddingProvider;
  /** A built-in embedding model (bge-m3-q8) or an Ollama one (bge-m3). */
  embeddingModel: string;
}

export interface AiSettingsView extends AiSettings {
  /** Providers that have an API key saved (the keys never come back). */
  keys: AiProvider[];
}

export interface CollectSettings {
  /** The AI reads what is collected for promises and contradictions. */
  analysis: boolean;
  /** Download the TSE's whole government-proposal file of a state (up to 1.4 GB) instead of reading only the candidate's PDF. */
  fullProposals: boolean;
  /** News pictures: loaded from their site, kept on this computer, or both (the copy, else the site). */
  newsImages: NewsImages;
}

export type NewsImages = 'url' | 'image' | 'both';

/** Optional web search service: Brave Search (a key) or a SearXNG (an address). */
export interface WebSearchSettings {
  provider: '' | 'brave' | 'searxng';
  endpoint: string;
  /** Empty: keep the saved one. */
  key: string;
  hasKey: boolean;
}

export const settingsService = {
  get: () => invoke<AiSettingsView>('get_ai_settings'),
  save: (settings: AiSettings) => invoke<void>('save_ai_settings', { settings }),
  /** An empty key removes it. */
  saveApiKey: (provider: AiProvider, key: string) => invoke<void>('save_api_key', { provider, key }),
  ollamaModels: (endpoint?: string) => invoke<string[]>('list_ollama_models', { endpoint: endpoint || null }),
  /** The models a provider offers to the key typed (or the one saved). */
  providerModels: (provider: AiProvider, key?: string, endpoint?: string) =>
    invoke<string[]>('list_provider_models', { provider, key: key || null, endpoint: endpoint || null }),
  /** Resolves with the model's answer when the configured model works. */
  test: () => invoke<string>('test_ai_model'),
  collect: () => invoke<CollectSettings>('get_collect_settings'),
  saveCollect: (settings: CollectSettings) => invoke<void>('save_collect_settings', { settings }),
  webSearch: () => invoke<WebSearchSettings>('get_websearch_settings'),
  saveWebSearch: (settings: WebSearchSettings) => invoke<void>('save_websearch_settings', { settings }),
  /** Resolves with how many results a search brought. */
  testWebSearch: (query: string) => invoke<number>('test_websearch', { query }),
};
