import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

/** politics: people, official data and claim analysis; interest: any other subject, only collected. */
export type InvestigationKind = 'politics' | 'interest';

export interface Investigation {
  id: string;
  name: string;
  description: string;
  kind: InvestigationKind;
  createdAt: string;
  documentCount: number;
  sourceCount: number;
}

export type TargetKind = 'person' | 'organization' | 'topic';

/** Someone in the official records (Câmara, Senado, ALESP, Transparência, TSE, a city hall, a state's amendments) who may be a target. */
export interface OfficialMatch {
  kind: 'camara_deputado' | 'senado_senador' | 'tse_candidato' | 'transparencia_emendas' | 'alesp_deputado' | 'pncp_prefeitura' | 'emendas_estaduais';
  /** The deputy id, the senator code, "year:UF:candidate id", the city's IBGE code, "UF:author"… */
  id: string;
  label: string;
  detail: string;
  /** Only shares words with the name searched (another first or last name): most likely someone else. */
  similar?: boolean;
}

export interface Target {
  id: string;
  investigationId: string;
  kind: TargetKind;
  name: string;
  aliases: string[];
  /** The official records it is linked to. */
  official: OfficialMatch[];
  createdAt: string;
}

export interface TargetInput {
  /** null: a new target. */
  id: string | null;
  investigationId: string;
  kind: TargetKind;
  name: string;
  aliases: string[];
}

export type SourceKind =
  | 'page'
  | 'rss'
  | 'youtube_channel'
  | 'youtube_video'
  | 'news_search'
  | 'youtube_search'
  | 'web_search'
  | 'gazette_search'
  | 'social'
  | 'camara_deputado'
  | 'senado_senador'
  | 'tse_candidato'
  | 'transparencia_emendas'
  | 'alesp_deputado'
  | 'pncp_prefeitura'
  | 'emendas_estaduais';

export interface Source {
  id: string;
  investigationId: string;
  targetId: string | null;
  kind: SourceKind;
  /** The page/feed/video address, or the query of a search. */
  url: string;
  title: string | null;
  everyMinutes: number;
  nextRunAt: string;
  lastRunAt: string | null;
  lastError: string | null;
  /** suggested: by the AI, waiting for approval; rejected: dismissed (never suggested again). */
  status: 'active' | 'paused' | 'done' | 'suggested' | 'rejected';
  /** target: one of a target's searches or official sources; llm: suggested by the AI. */
  origin: 'user' | 'target' | 'llm';
  /** The links of what it brings are followed. */
  followLinks: boolean;
  /** Why the AI suggested it. */
  note: string | null;
  createdAt: string;
  documentCount: number;
  queued: number;
}

export type DocumentKind = 'article' | 'page' | 'video' | 'post' | 'live' | 'speech' | 'bill' | 'vote' | 'expense' | 'amendment' | 'contract' | 'procurement' | 'gazette' | 'profile' | 'candidacy' | 'proposal';

/** Records published by the institutions themselves (the primary source); the rest is secondary. */
export const RECORD_KINDS: DocumentKind[] = ['speech', 'bill', 'vote', 'expense', 'profile', 'candidacy', 'proposal', 'amendment', 'contract', 'procurement', 'gazette'];

export const DOCUMENT_KINDS: DocumentKind[] = ['article', 'video', 'post', 'live', 'speech', 'vote', 'bill', 'amendment', 'expense', 'contract', 'procurement', 'gazette', 'proposal', 'candidacy', 'profile', 'page'];

export interface DocumentSummary {
  id: string;
  investigationId: string;
  sourceId: string | null;
  url: string;
  kind: DocumentKind;
  title: string;
  author: string | null;
  site: string | null;
  publishedAt: string | null;
  fetchedAt: string;
  /** Why the text is incomplete (e.g. no subtitles). */
  note: string | null;
  indexError: string | null;
}

export interface TimedText {
  start: number;
  end: number;
  text: string;
}

export interface VogaDocument extends DocumentSummary {
  text: string;
  segments: TimedText[] | null;
  starred: boolean;
  /** -1: less of this; 1: more of this (the Feed's relevance). */
  feedback: Feedback;
}

export type Feedback = -1 | 0 | 1;

export interface Page<T> {
  items: T[];
  total: number;
}

export interface CollectProgress {
  running: boolean;
  current: string | null;
  queued: number;
  collected: number;
  /** The AI is reading documents (`current` is the one). */
  analyzing: boolean;
  /** Whose source or page `current` is. */
  investigationId: string | null;
  /** Per investigation, how far this run is. */
  runs: { investigationId: string; done: number; total: number }[];
}

export const investigationService = {
  list: () => invoke<Investigation[]>('list_investigations'),
  create: (name: string, description: string, kind: InvestigationKind) => invoke<Investigation>('create_investigation', { name, description, kind }),
  update: (id: string, name: string, description: string) => invoke<Investigation>('update_investigation', { id, name, description }),
  /** Turns it into an interest or back: its searches follow, and the AI's pending suggestions go. */
  setKind: (id: string, kind: InvestigationKind) => invoke<Investigation>('set_investigation_kind', { id, kind }),
  remove: (id: string) => invoke<void>('delete_investigation', { id }),

  targets: (investigationId: string) => invoke<Target[]>('list_targets', { investigationId }),
  saveTarget: (target: TargetInput) => invoke<string>('save_target', { target }),
  deleteTarget: (id: string) => invoke<void>('delete_target', { id }),
  /** Searches the Câmara, the Senado and the TSE (general elections of `years`) for the target. */
  findOfficial: (targetId: string, years?: number[]) => invoke<OfficialMatch[]>('find_official_matches', { targetId, years: years ?? null }),
  linkOfficial: (targetId: string, matches: OfficialMatch[]) => invoke<void>('link_official', { targetId, matches }),

  sources: (investigationId: string) => invoke<Source[]>('list_sources', { investigationId }),
  addSource: (investigationId: string, kind: SourceKind, url: string, title?: string) =>
    invoke<string>('add_source', { investigationId, kind, url, title: title || null }),
  updateSource: (id: string, changes: { status?: 'active' | 'paused'; everyMinutes?: number; followLinks?: boolean }) =>
    invoke<void>('update_source', {
      id,
      status: changes.status ?? null,
      everyMinutes: changes.everyMinutes ?? null,
      followLinks: changes.followLinks ?? null,
    }),
  deleteSource: (id: string) => invoke<void>('delete_source', { id }),
  runSourceNow: (id: string) => invoke<void>('run_source_now', { id }),

  /** The documents of one investigation, or of several open together. */
  documents: (investigationIds: string[], options: { kind?: DocumentKind; limit?: number; offset?: number } = {}) =>
    invoke<Page<DocumentSummary>>('list_documents', {
      investigationIds,
      kind: options.kind ?? null,
      limit: options.limit ?? null,
      offset: options.offset ?? null,
    }),
  document: (id: string) => invoke<VogaDocument>('get_document', { id }),
  deleteDocument: (id: string) => invoke<void>('delete_document', { id }),

  collectStatus: () => invoke<CollectProgress>('collect_status'),
  onProgress: (callback: (progress: CollectProgress) => void): Promise<UnlistenFn> =>
    listen<CollectProgress>('collect-progress', event => callback(event.payload)),
};

/** An investigation's discovery limits (per day). */
export interface DiscoveryLimits {
  /** Pages queued from followed links. */
  pagesPerDay: number;
  /** AI requests discovery makes (choosing links, suggesting searches). */
  aiCallsPerDay: number;
  /** How many links away from a source (0: links are not followed; at most 2). */
  depth: number;
  /** The AI's suggestions start without waiting for approval. */
  autoApprove: boolean;
}

export interface Discovery {
  limits: DiscoveryLimits;
  /** Spent today. */
  usage: { pages: number; aiCalls: number };
}

export const discoveryService = {
  get: (investigationId: string) => invoke<Discovery>('get_discovery', { investigationId }),
  save: (investigationId: string, limits: DiscoveryLimits) => invoke<void>('save_discovery', { investigationId, limits }),
  approve: (id: string) => invoke<void>('approve_source', { id }),
  reject: (id: string) => invoke<void>('reject_source', { id }),
  /** Asks the AI for searches now; resolves with how many it suggested. */
  suggest: (investigationId: string) => invoke<number>('suggest_sources', { investigationId }),
};

/** Minutes between runs for each group of sources; null keeps each source's own pace. */
export interface ScheduleEvery {
  /** RSS feeds and social profiles. */
  feeds: number | null;
  /** News and web searches. */
  news: number | null;
  /** YouTube searches and channels. */
  videos: number | null;
  pages: number | null;
  /** Municipal gazettes. */
  gazettes: number | null;
  /** Official records (the PNCP city hall keeps its hourly catch-up). */
  official: number | null;
}

/** When an investigation collects. */
export interface CollectSchedule {
  /** Nothing of it is collected (sources, queued pages) until resumed; "run now" still works. */
  paused: boolean;
  every: ScheduleEvery;
  /** Local hours "HH:MM", across midnight when `end` comes first; null: any time. */
  window: { start: string; end: string } | null;
  /** Days of the week, 0 = Sunday; empty: every day. */
  days: number[];
}

export interface ScheduleView {
  schedule: CollectSchedule;
  /** It may collect right now. */
  open: boolean;
}

export const scheduleService = {
  get: (investigationId: string) => invoke<ScheduleView>('get_collect_schedule', { investigationId }),
  save: (investigationId: string, schedule: CollectSchedule) => invoke<void>('save_collect_schedule', { investigationId, schedule }),
};

/** The address to open for a document, at `seconds` into a video. */
export function documentLink(url: string, seconds?: number | null): string {
  if (seconds == null || !url.includes('youtube.com/watch')) return url;
  return `${url}&t=${Math.floor(seconds)}s`;
}

export function formatClock(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
}
