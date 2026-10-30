import { invoke } from '@tauri-apps/api/core';
import type { DocumentKind } from './investigationService';

export interface Citation {
  /** The [n] number used in the answer. */
  index: number;
  chunkId: string;
  documentId: string;
  title: string;
  url: string;
  site: string | null;
  kind: DocumentKind;
  docDate: string;
  /** Seconds into a video. */
  startTime: number | null;
  excerpt: string;
}

export interface QueryPlan {
  searchQuery: string;
  dateFrom: string | null;
  dateTo: string | null;
  kind: DocumentKind | null;
}

export interface Answer {
  answer: string;
  /** False when no relevant passage was found. */
  found: boolean;
  citations: Citation[];
  plan: QueryPlan;
  filtersRelaxed: boolean;
  /** Set when semantic search was unavailable (keyword results only). */
  notice: string | null;
  historyId: string | null;
}

export interface ConversationTurn {
  question: string;
  answer: string;
}

export interface AskHistoryEntry {
  id: string;
  question: string;
  createdAt: string;
}

export interface SearchHit {
  chunkId: string;
  documentId: string;
  title: string;
  url: string;
  kind: DocumentKind;
  site: string | null;
  docDate: string;
  text: string;
  startTime: number | null;
  endTime: number | null;
  score: number;
  sources: ('vector' | 'lexical')[];
}

export interface SearchResponse {
  results: SearchHit[];
  vectorError: string | null;
}

export const askService = {
  /** Asks about one investigation, or several open together. */
  ask: (investigationIds: string[], question: string, history: ConversationTurn[]) =>
    invoke<Answer>('ask_question', { request: { investigationIds, question, history } }),
  history: (investigationIds: string[]) => invoke<AskHistoryEntry[]>('list_ask_history', { investigationIds }),
  saved: (id: string) => invoke<Answer | null>('get_ask_answer', { id }),
  search: (investigationIds: string[], query: string, limit = 20) => invoke<SearchResponse>('search_documents', { investigationIds, query, limit }),
};
