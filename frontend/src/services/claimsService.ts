import { invoke } from '@tauri-apps/api/core';
import type { DocumentKind } from './investigationService';

export interface DocRef {
  documentId: string | null;
  title: string | null;
  url: string | null;
  kind: DocumentKind | null;
  site: string | null;
}

export type FindingStatus = 'suggested' | 'confirmed' | 'rejected';

/** What the AI found about a claim in another record. */
export interface Finding {
  id: string;
  label: 'CONTRADICTS' | 'FULFILLS';
  status: FindingStatus;
  confidence: number;
  rationale: string | null;
  /** Date of the record. */
  date: string;
  excerpt: string | null;
  startTime: number | null;
  evidence: DocRef;
  /** Whose act the evidence is (null: a report, such as news or a video). */
  actor: Actor | null;
}

/** Who did what a record shows, next to who made the claim. */
export interface Actor {
  targetId: string;
  name: string;
  party: string | null;
  /** The claimant themselves, someone of their party, of their coalition, or someone else. */
  scope: 'self' | 'party' | 'ally' | 'other';
}

/** A finding about the claimant's own act (or a report): only these decide a claim's state. */
export const ownFinding = (finding: Finding) => !finding.actor || finding.actor.scope === 'self';

/** A promise or statement a person followed made, with the words it came from. */
export interface Claim {
  id: string;
  kind: 'promise' | 'statement';
  text: string;
  quote: string;
  topic: string;
  statedAt: string;
  targetId: string;
  startTime: number | null;
  source: DocRef;
  findings: Finding[];
}

export interface AnalysisStatus {
  pending: number;
  analyzed: number;
  failed: number;
}

export type ClaimState = 'contradicted' | 'fulfilled' | 'review' | 'none';

/** Where a claim stands: confirmed findings about the person's own acts decide (another
 * person's act is at most their party's or group's); suggestions wait for review. */
export function claimState(claim: Claim): ClaimState {
  const live = claim.findings.filter(f => f.status !== 'rejected');
  if (live.some(f => ownFinding(f) && f.label === 'CONTRADICTS' && f.status === 'confirmed')) return 'contradicted';
  if (live.some(f => ownFinding(f) && f.label === 'FULFILLS' && f.status === 'confirmed')) return 'fulfilled';
  if (live.some(f => f.status === 'suggested')) return 'review';
  return 'none';
}

export const claimsService = {
  list: (investigationId: string) => invoke<Claim[]>('list_claims', { investigationId }),
  setStatus: (id: string, status: FindingStatus) => invoke<void>('set_finding_status', { id, status }),
  remove: (id: string) => invoke<void>('delete_claim', { id }),
  status: (investigationId: string) => invoke<AnalysisStatus>('analysis_status', { investigationId }),
};
