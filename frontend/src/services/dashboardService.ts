import { invoke } from '@tauri-apps/api/core';
import type { CollectSchedule, InvestigationKind } from './investigationService';

/** Claims a person confirmed, and claims with an AI finding waiting for review. */
export interface Tally {
  confirmed: number;
  suggested: number;
}

/** The latest thing found about the people an investigation follows. */
export interface Highlight {
  kind: 'contradiction' | 'fulfillment' | 'check_false' | 'check_true' | 'scandal';
  text: string;
  person: string | null;
  /** When it was found. */
  date: string;
  /** A suggestion is never shown as a fact. */
  status: 'confirmed' | 'suggested';
  documentId: string | null;
  sessionId: string | null;
}

export interface ScheduleState {
  schedule: CollectSchedule;
  /** It may collect now. */
  open: boolean;
  /** When it may collect again (local time, YYYY-MM-DDTHH:MM:00), while closed by its hours or days. */
  nextOpen: string | null;
  nextRunAt: string | null;
}

/** An investigation at a glance (Home). */
export interface InvestigationStatus {
  id: string;
  name: string;
  kind: InvestigationKind;
  targets: string[];
  sources: number;
  failing: number;
  documents: number;
  unread: number;
  lastUpdate: string | null;
  indexing: number;
  queued: number;
  schedule: ScheduleState;
  contradictions: Tally;
  fulfilled: Tally;
  /** AI findings and suggested sources waiting for review. */
  insights: number;
  latest: Highlight | null;
  /** Estimate of the room it takes on this computer. */
  diskBytes: number;
}

/** One person followed, by what a person confirmed about them; what waits for review is apart. */
export interface PersonScore {
  targetId: string;
  name: string;
  investigationId: string;
  investigationName: string;
  /** Promises and statements confirmed contradicted by their own acts. */
  contradictions: number;
  /** Live statements confirmed false. */
  falseChecks: number;
  fulfilled: number;
  /** Live statements confirmed true. */
  trueChecks: number;
  /** Scandals they were confirmed involved in. */
  scandals: number;
  /** Sanctioned companies they are tied to (public records). */
  sanctioned: number;
  toReview: number;
}

export const dashboardService = {
  load: () => invoke<InvestigationStatus[]>('dashboard'),
  /** Everyone followed, the most contradictions and false statements first. */
  scoreboard: () => invoke<PersonScore[]>('scoreboard'),
};
