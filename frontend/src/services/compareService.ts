import { invoke } from '@tauri-apps/api/core';
import type { Citation } from './askService';
import type { InvestigationKind } from './investigationService';

export interface Side {
  id: string;
  name: string;
  kind: InvestigationKind;
  documents: number;
  targets: string[];
}

export interface SameDocument {
  url: string;
  title: string;
  documentId: string;
  sides: string[];
}

/** How a side's entity is tied to what is shared ("Marina Exemplo" · MEMBER_OF). */
export interface SharedTie {
  investigationId: string;
  who: string;
  label: string;
  date: string;
}

export interface Shared {
  key: string;
  kind: string;
  label: string;
  ties: SharedTie[];
}

export interface VoteDiff {
  label: string;
  date: string;
  documentId: string | null;
  votes: { investigationId: string; who: string; vote: string }[];
}

export interface Votes {
  /** Votes every side's people voted in. */
  both: number;
  agreed: number;
  disagreements: VoteDiff[];
}

export interface TopicSide {
  investigationId: string;
  topic: string;
  claims: number;
  contradicted: number;
  fulfilled: number;
}

/** A passage and how close it is to each side's interest (in `sides` order). */
export interface Passage {
  investigationId: string;
  documentId: string;
  chunkId: string;
  title: string;
  site: string | null;
  date: string;
  scores: number[];
}

export interface Comparison {
  sides: Side[];
  sameDocuments: SameDocument[];
  shared: Shared[];
  votes: Votes;
  topics: { sides: TopicSide[] }[];
  /** Where they connect. */
  bridges: Passage[];
  /** Where they diverge: each side's passages furthest from the others. */
  divergent: Passage[];
  /** Documents per week, per side (in `sides` order). */
  timeline: { week: string; counts: number[] }[];
  notice: string | null;
}

export interface CompareSummary {
  /** Markdown: where they connect, where they diverge, what is missing; [n] cites `citations`. */
  text: string;
  citations: Citation[];
}

export const compareService = {
  compare: (investigationIds: string[]) => invoke<Comparison>('compare_investigations', { investigationIds }),
  summary: (investigationIds: string[]) => invoke<CompareSummary>('compare_summary', { investigationIds }),
};
