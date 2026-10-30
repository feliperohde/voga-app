import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export type NodeKind =
  | 'person'
  | 'party'
  | 'organization'
  | 'bill'
  | 'vote_session'
  | 'candidacy'
  | 'promise'
  | 'statement'
  /** A collected document a claim is tied to (its kind: article, speech, video…). */
  | string;

export interface GraphNode {
  id: string;
  kind: NodeKind;
  key: string;
  label: string;
  /** JSON text: official data and public data looked up (cnpj, razao_social, sanctions…). */
  attrs: string;
  documentId: string | null;
  /** On a map of several investigations: the ones it is in (shared when more than one). */
  investigations: string[];
}

export interface GraphLink {
  id: string;
  source: string;
  target: string;
  /** MEMBER_OF, AUTHORED, VOTED, ABOUT, SPENT_WITH, CANDIDATE_IN, MADE, CONTRADICTS, FULFILLS, PARTNER_OF, PAID_TO, CONTRACTED, FUNDS, HEADS */
  label: string;
  status: 'confirmed' | 'suggested';
  date: string;
  /** The vote, for VOTED. */
  vote: string | null;
  /** JSON text: amount (SPENT_WITH, PAID_TO), valor and objeto (CONTRACTED, FUNDS), role (PARTNER_OF)… */
  props: string;
  /** The record that shows it. */
  documentId: string | null;
}

export interface Graph {
  nodes: GraphNode[];
  links: GraphLink[];
}

export interface GraphOptions {
  /** Every nominal vote, not only those tied to a claim. */
  allVotes: boolean;
  /** The AI's findings not reviewed yet. */
  suggested: boolean;
}

export interface FlowsintSettings {
  /** local (default): flowsint on this computer, through Docker | server: one you run elsewhere. */
  mode: 'local' | 'server';
  endpoint: string;
  email: string;
  /** Empty: keep the saved one. */
  password: string;
  hasPassword: boolean;
}

export interface FlowsintProgress {
  stage: 'pull' | 'start' | 'account';
  /** The last line docker printed. */
  message: string;
}

/** flowsint on this computer. */
export interface LocalFlowsint {
  /** missing: no Docker with Compose; stopped: Docker is not running. */
  engine: 'missing' | 'stopped' | 'running';
  stack: 'absent' | 'stopped' | 'starting' | 'running';
  installed: boolean;
  url: string | null;
  email: string;
  password: string | null;
  /** Docker Desktop or OrbStack, when Voga can open it. */
  engineApp: string | null;
  busy: boolean;
  progress: FlowsintProgress | null;
  version: string;
}

export interface FlowsintSync {
  investigation: string;
  sketch: string;
  pushedAt: string;
  url: string;
}

export interface PushResult {
  url: string;
  nodes: number;
  edges: number;
  nodesCreated: number;
  errors: string[];
}

export const graphService = {
  load: (investigationIds: string[], options: GraphOptions) => invoke<Graph>('investigation_graph', { investigationIds, options }),
  /** Looks up a company's Receita record, partners and federal sanctions again. */
  enrich: (entityId: string) => invoke<void>('enrich_entity', { entityId }),
};

export const flowsintService = {
  settings: () => invoke<FlowsintSettings>('get_flowsint_settings'),
  save: (settings: FlowsintSettings) => invoke<void>('save_flowsint_settings', { settings }),
  test: () => invoke<void>('test_flowsint'),
  createAccount: () => invoke<void>('create_flowsint_account'),
  push: (investigationId: string) => invoke<PushResult>('push_to_flowsint', { investigationId }),
  sync: (investigationId: string) => invoke<FlowsintSync | null>('flowsint_sync', { investigationId }),
  localStatus: () => invoke<LocalFlowsint>('flowsint_local_status'),
  /** Downloads the images (a few GB), starts flowsint and creates Voga's account. */
  install: () => invoke<void>('flowsint_local_install'),
  start: () => invoke<void>('flowsint_local_start'),
  stop: () => invoke<void>('flowsint_local_stop'),
  /** Deletes the local flowsint with its data and images. */
  remove: () => invoke<void>('flowsint_local_remove'),
  openDocker: () => invoke<void>('open_docker'),
  onProgress: (handler: (progress: FlowsintProgress) => void): Promise<UnlistenFn> =>
    listen<FlowsintProgress>('flowsint-local-progress', event => handler(event.payload)),
};
