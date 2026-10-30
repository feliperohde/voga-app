import { invoke } from '@tauri-apps/api/core';

/** Someone involved in a scandal: marked by a person (confirmed), or suggested by the AI from a news item. */
export interface Involved {
  /** Reviewed with claimsService.setStatus when the AI suggested it. */
  relationId: string;
  targetId: string | null;
  name: string;
  status: 'suggested' | 'confirmed' | 'rejected';
  origin: 'user' | 'llm';
  /** The news' words (an AI suggestion). */
  rationale: string | null;
}

export interface Scandal {
  id: string;
  investigationId: string;
  title: string;
  date: string | null;
  documentId: string | null;
  documentTitle: string | null;
  people: Involved[];
}

/** One way a person is tied to a sanctioned company (public records). */
export interface Tie {
  /** SPENT_WITH, or PAID_TO / FUNDS / CONTRACTED through `via` (an amendment, a city hall). */
  label: string;
  via: string | null;
  date: string;
  props: Record<string, unknown>;
  documentId: string | null;
}

export interface Sanction {
  cadastro: string;
  categoria: string;
  orgao: string;
  inicio: string;
  fim: string;
  processo: string;
}

export interface SanctionedTie {
  investigationId: string;
  targetId: string;
  person: string;
  companyId: string;
  company: string;
  sanctions: Sanction[];
  ties: Tie[];
}

export interface Scandals {
  scandals: Scandal[];
  sanctioned: SanctionedTie[];
}

export const scandalService = {
  list: (investigationId: string) => invoke<Scandals>('list_scandals', { investigationId }),
  /** Marks a document as a scandal involving these people (followed in its investigation). */
  mark: (documentId: string, title: string, date: string, targetIds: string[]) => invoke<string>('mark_scandal', { documentId, title, date, targetIds }),
  remove: (id: string) => invoke<void>('delete_scandal', { id }),
};
