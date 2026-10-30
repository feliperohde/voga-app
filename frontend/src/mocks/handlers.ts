/**
 * Mock mode: the app commands of the Rust core, answered from the demo state below
 * (in memory, reset on reload). A new Tauri command the frontend uses needs a handler here.
 */

import { emit } from '@tauri-apps/api/event';
import type { Answer } from '@/services/askService';
import type { BuiltinModel } from '@/services/builtinService';
import type { Claim } from '@/services/claimsService';
import type { InvestigationStatus, PersonScore } from '@/services/dashboardService';
import type { Scandals } from '@/services/scandalService';
import type { Comparison, CompareSummary } from '@/services/compareService';
import type { CollectSchedule, DocumentSummary, Investigation, OfficialMatch, SourceKind } from '@/services/investigationService';
import type { AiProvider, AiSettings, NewsImages } from '@/services/settingsService';
import { CLAIMS, DemoDocument, DOCUMENTS, INVESTIGATIONS, OFFICIAL_MATCHES, SOURCES, TARGETS } from './data';
import { LIVE_MODELS, liveHandlers } from './live';

type Handler = (args: Record<string, any>) => unknown;

/** The built-in catalog, as the Rust core lists it; the demo has the recommended ones downloaded. */
const BUILTIN_MODELS: BuiltinModel[] = [
  { kind: 'summary', name: 'qwen3.5:4b', display_name: 'Qwen 3.5 4B (High Quality)', size_mb: 2614, status: { type: 'available' },
    description: 'Recommended: the best answers and analysis of the built-in models (~4 GB of memory).' },
  { kind: 'summary', name: 'qwen3.5:2b', display_name: 'Qwen 3.5 2B (Balanced)', size_mb: 1221, status: { type: 'not_downloaded' },
    description: 'Balanced: good Portuguese with modest requirements (~2 GB of memory).' },
  { kind: 'summary', name: 'gemma3:4b', display_name: 'Gemma 3 4B (Balanced)', size_mb: 2374, status: { type: 'not_downloaded' },
    description: 'Alternative to Qwen with a good quality/speed balance (~3.5 GB of memory).' },
  { kind: 'summary', name: 'gemma3:1b', display_name: 'Gemma 3 1B (Fast)', size_mb: 1019, status: { type: 'not_downloaded' },
    description: 'The fastest: runs on any computer (~1 GB of memory), with simpler answers.' },
  { kind: 'embedding', name: 'bge-m3-q8', display_name: 'BGE-M3 (Multilingual)', size_mb: 605, status: { type: 'available' },
    description: 'Recommended. Multilingual (good for Portuguese): finds documents by meaning.' },
  { kind: 'embedding', name: 'nomic-embed-text-v1.5-q8', display_name: 'Nomic Embed Text v1.5 (Compact)', size_mb: 139, status: { type: 'not_downloaded' },
    description: 'Small and fast. Best for English sources.' },
];

/** Downloads in progress: a timer that advances them, and how to stop one. */
const downloads = new Map<string, { timer: ReturnType<typeof setInterval>; cancel: () => void }>();

const progressEvent = (payload: Record<string, unknown>) => emit('builtin-ai-download-progress', payload).catch(() => {});

/** A few seconds of progress events, like a real download. */
function simulateDownload(name: string): Promise<void> {
  const model = state.builtin.find(m => m.name === name);
  if (!model) return Promise.reject(`Unknown model: ${name}`);
  return new Promise((resolve, reject) => {
    let progress = 0;
    model.status = { type: 'downloading', progress };
    const timer = setInterval(() => {
      progress = Math.min(100, progress + 5);
      model.status = { type: 'downloading', progress };
      progressEvent({ model: name, progress, downloaded_mb: (model.size_mb * progress) / 100, total_mb: model.size_mb, speed_mbps: 48.5, status: 'downloading' });
      if (progress === 100) {
        clearInterval(timer);
        downloads.delete(name);
        model.status = { type: 'available' };
        progressEvent({ model: name, progress: 100, status: 'completed' });
        resolve();
      }
    }, 150);
    downloads.set(name, {
      timer,
      cancel: () => {
        clearInterval(timer);
        downloads.delete(name);
        model.status = { type: 'not_downloaded' };
        reject('CANCELLED: Download cancelled by user');
      },
    });
  });
}

const LOCAL_FLOWSINT = 'http://127.0.0.1:5183';

/** The local flowsint's install or start: its stages, a second each. */
function simulateFlowsint(stages: string[]): Promise<void> {
  state.flowsintLocal.busy = true;
  return new Promise(resolve => {
    stages.forEach((stage, i) =>
      setTimeout(() => {
        emit('flowsint-local-progress', { stage, message: stage === 'pull' ? 'neo4j Pulling' : '' }).catch(() => {});
      }, i * 1000),
    );
    setTimeout(() => {
      state.flowsintLocal = { installed: true, running: true, busy: false };
      resolve();
    }, stages.length * 1000);
  });
}

/** Demo of a modest computer (localStorage voga.mock.basic = 1): no built-in model downloaded. */
const mockBasic = () => {
  try {
    return localStorage.getItem('voga.mock.basic') === '1';
  } catch {
    return false;
  }
};

/** Demo of the first run (localStorage voga.mock.fresh = 1): no investigation yet, so Home shows the wizard. */
const mockFresh = () => {
  try {
    return localStorage.getItem('voga.mock.fresh') === '1';
  } catch {
    return false;
  }
};

/** What each provider lists to a key (their real model ids, as of the demo). */
const PROVIDER_MODELS: Record<string, string[]> = {
  openai: ['gpt-6', 'gpt-6-mini', 'text-embedding-3-large', 'text-embedding-3-small', 'gpt-4o-mini-transcribe', 'whisper-1', 'gpt-4o-mini-tts'],
  gemini: ['gemini-3.8-flash', 'gemini-3.8-flash-lite', 'gemini-3-pro', 'gemini-embedding-2'],
  groq: ['openai/gpt-oss-120b', 'llama-3.3-70b-versatile', 'whisper-large-v3', 'whisper-large-v3-turbo'],
  deepseek: ['deepseek-v4-flash', 'deepseek-v4-pro'],
  claude: ['claude-haiku-5-5', 'claude-sonnet-5-5', 'claude-opus-5-5'],
  openrouter: ['openai/gpt-oss-120b', 'qwen/qwen3.8-27b'],
  'custom-openai': ['qwen3.8-27b', 'bge-m3', 'faster-whisper-large-v3'],
};

const state = {
  ai: {
    provider: 'builtin-ai',
    model: 'qwen3.5:4b',
    ollamaEndpoint: null,
    customEndpoint: null,
    embeddingProvider: 'builtin',
    embeddingModel: 'bge-m3-q8',
  } as AiSettings,
  builtin: structuredClone([...BUILTIN_MODELS, ...LIVE_MODELS]).map(m =>
    mockBasic() && (m.kind === 'summary' || m.kind === 'embedding') ? { ...m, status: { type: 'not_downloaded' as const } } : m,
  ),
  keys: [] as AiProvider[],
  analysis: true,
  fullProposals: false,
  newsImages: 'both' as NewsImages,
  flowsint: { mode: 'local', endpoint: '', email: '', password: '', hasPassword: false } as {
    mode: 'local' | 'server';
    endpoint: string;
    email: string;
    password: string;
    hasPassword: boolean;
  },
  /** The demo's flowsint on this computer: installed and running. */
  flowsintLocal: { installed: true, running: true, busy: false },
  discovery: { pagesPerDay: 100, aiCallsPerDay: 20, depth: 1, autoApprove: false },
  /** Each investigation's collection schedule (default until saved). */
  // Fictional: a case the AI suggests about the mayor, and a sanctioned supplier tied to the deputy
  scandals: {
    'inv-prefeita': {
      scandals: [
        {
          id: 'scandal-merenda',
          investigationId: 'inv-prefeita',
          title: 'Operação Ponte Quebrada',
          date: new Date(Date.now() - 30 * 3600000).toISOString(),
          documentId: 'doc-merenda',
          documentTitle: 'Operação Ponte Quebrada investiga compra de merenda em Cidade Exemplo',
          people: [
            { relationId: 'rel-merenda', targetId: 'target-helena', name: 'Helena Prado', status: 'suggested', origin: 'llm', rationale: 'a prefeita Helena Prado é investigada por fraude na compra de merenda escolar' },
          ],
        },
      ],
      sanctioned: [],
    },
    'inv-demo': {
      scandals: [],
      sanctioned: [
        {
          investigationId: 'inv-demo',
          targetId: 'target-marina',
          person: 'Marina Exemplo',
          companyId: 'n-grafica',
          company: 'Gráfica Exemplo Ltda',
          sanctions: [{ cadastro: 'CEIS', categoria: 'Impedimento de contratar com prazo determinado', orgao: 'Prefeitura de Cidade Vizinha', inicio: '10/03/2025', fim: '10/03/2027', processo: '' }],
          ties: [
            { label: 'SPENT_WITH', via: null, date: '2026-08', props: { amount: 12200 }, documentId: 'doc-cota' },
            { label: 'FUNDS', via: 'Emenda 0003/2026 (Educação)', date: '', props: {}, documentId: null },
          ],
        },
      ],
    },
  } as Record<string, Scandals>,
  // The city hall collects only at night (Home shows it waiting)
  schedules: {
    'inv-prefeita': { paused: false, every: { feeds: null, news: null, videos: null, pages: null, gazettes: null, official: null }, window: { start: '22:00', end: '06:00' }, days: [] },
  } as Record<string, CollectSchedule>,
  webSearch: { provider: '', endpoint: '', key: '', hasKey: false } as { provider: '' | 'brave' | 'searxng'; endpoint: string; key: string; hasKey: boolean },
  flowsintSync: null as null | { investigation: string; sketch: string; pushedAt: string; url: string },
  /** What "Fetch public data" found about the demo print shop (empty until pressed). */
  graficaPublicData: {} as Record<string, unknown>,
  claims: structuredClone(CLAIMS),
  investigations: mockFresh() ? [] : [...INVESTIGATIONS],
  targets: [...TARGETS],
  sources: [...SOURCES],
  documents: structuredClone(DOCUMENTS),
  asked: [] as { id: string; investigationId: string; question: string; createdAt: string; answer: Answer }[],
};

const now = () => new Date().toISOString();
const id = (prefix: string) => `${prefix}-${Math.random().toString(36).slice(2, 10)}`;

function withCounts(investigation: Investigation): Investigation {
  return {
    ...investigation,
    documentCount: state.documents.filter(d => docInvestigation(d) === investigation.id).length,
    sourceCount: state.sources.filter(s => s.investigationId === investigation.id).length,
  };
}

function docInvestigation(document: DemoDocument): string {
  return state.sources.find(s => s.id === document.sourceId)?.investigationId ?? 'inv-demo';
}

function summary(document: DemoDocument): DocumentSummary {
  const { text: _text, segments: _segments, ...rest } = document;
  return { ...rest, investigationId: docInvestigation(document) };
}

const docDate = (d: DemoDocument) => d.publishedAt ?? d.fetchedAt;

/** What the Feed lists (not the official records). */
const FEED_KINDS = ['article', 'page', 'video', 'post'];

/** The demo's answer: cites the campaign promise and the vote against it. */
function demoAnswer(question: string): Answer {
  const promise = state.documents.find(d => d.id === 'doc-comicio');
  const vote = state.documents.find(d => d.id === 'doc-voto');
  const citations = [promise, vote]
    .filter((d): d is DemoDocument => !!d)
    .map((d, i) => ({
      index: i + 1,
      chunkId: `chunk-${d.id}`,
      documentId: d.id,
      title: d.title,
      url: d.url,
      site: d.site,
      kind: d.kind,
      docDate: docDate(d),
      startTime: d.segments ? 66 : null,
      excerpt: d.segments ? d.segments.slice(1, 3).map(s => s.text).join(' ') : d.text.slice(0, 300),
    }));
  return {
    answer: `Na campanha de 2022, num comício, Marina Exemplo disse que votaria "pela simplificação dos impostos" [1]. Em ${new Date(docDate(vote!)).toLocaleDateString('pt-BR')}, segundo o Jornal Fictício, ela votou contra o texto-base da reforma tributária; a assessoria atribuiu a mudança a alterações do relator [2].\n\n*Demonstração: pessoas e veículos fictícios. Pergunta: ${question}*`,
    found: citations.length > 0,
    citations,
    plan: { searchQuery: question, dateFrom: null, dateTo: null, kind: null },
    filtersRelaxed: false,
    notice: null,
    historyId: null,
  };
}

const OLLAMA_MODELS = ['bge-m3:latest', 'gemma3:12b', 'qwen3:14b'];

/** The fictional deputy's connections, built from the demo claims (their findings' review status counts). */
function demoGraph(allVotes: boolean, suggested: boolean) {
  const node = (id: string, kind: string, label: string, documentId: string | null = null, attrs: object = {}) => ({
    id, kind, key: id, label, documentId, attrs: JSON.stringify(attrs),
  });
  const nodes = [
    node('n-marina', 'person', 'Marina Exemplo'),
    node('n-pde', 'party', 'PDE'),
    node('n-cand', 'candidacy', 'DEPUTADO FEDERAL 2022 (PDE 9090)', 'doc-candidatura'),
    node('n-pl1234', 'bill', 'PL 1234/2025', 'doc-projeto'),
    node('n-vot', 'vote_session', 'PLP 99/2025 (02/10/2026)', 'doc-votacao-oficial'),
    node('n-grafica', 'organization', 'Gráfica Exemplo Ltda', null, { cnpj: '11222333000181', ...state.graficaPublicData }),
    node('n-aerea', 'organization', 'Linhas Aéreas Fictícias S.A.', null, { cnpj: '44555666000199' }),
    ...(state.graficaPublicData.razao_social ? [node('n-socio', 'person', 'Bruno Fictício')] : []),
    node('n-emenda', 'amendment', 'Emenda 0003/2026 (Educação)'),
    node('n-cidade', 'organization', 'Município de Cidade Exemplo', null, { cnpj: '99000111000122' }),
    node('n-prefeitura', 'organization', 'Prefeitura de Cidade Exemplo', 'doc-contratos'),
    ...state.claims.map(c => node(c.id, c.kind, c.text, c.source.documentId)),
  ];
  if (allVotes) nodes.push(node('n-vot2', 'vote_session', 'PL 777/2023 (12/04/2023)'));
  const link = (source: string, target: string, label: string, status = 'confirmed', vote: string | null = null, date = '', props: object = {}, documentId: string | null = null) => ({
    id: `${source}-${target}-${label}-${date}-${JSON.stringify(props)}`, source, target, label, status, date, vote, props: JSON.stringify(props), documentId,
  });
  const links = [
    link('n-marina', 'n-pde', 'MEMBER_OF'),
    link('n-marina', 'n-cand', 'CANDIDATE_IN'),
    link('n-marina', 'n-pl1234', 'AUTHORED'),
    link('n-marina', 'n-vot', 'VOTED', 'confirmed', 'Não'),
    link('n-marina', 'n-grafica', 'SPENT_WITH', 'confirmed', null, '2026-07', { amount: 9800 }, 'doc-cota'),
    link('n-marina', 'n-grafica', 'SPENT_WITH', 'confirmed', null, '2026-08', { amount: 12200 }, 'doc-cota'),
    link('n-marina', 'n-aerea', 'SPENT_WITH'),
    link('n-socio', 'n-grafica', 'PARTNER_OF'),
    link('n-marina', 'n-emenda', 'AUTHORED'),
    link('n-emenda', 'n-cidade', 'PAID_TO'),
    link('n-emenda', 'n-grafica', 'FUNDS'),
    link('n-prefeitura', 'n-grafica', 'CONTRACTED', 'confirmed', null, '2026-08-20', { valor: 250500, objeto: 'Impressão de material didático para creches' }, 'doc-contratos'),
    ...state.claims.map(c => link('n-marina', c.id, 'MADE')),
    ...state.claims.flatMap(c =>
      c.findings
        .filter(f => f.status !== 'rejected' && (suggested || f.status === 'confirmed'))
        .map(f => link(c.id, f.evidence.documentId === 'doc-projeto' ? 'n-pl1234' : 'n-vot', f.label, f.status, null, f.date, { scope: f.actor?.scope ?? 'none', finding: f.id })),
    ),
  ];
  if (allVotes) links.push(link('n-marina', 'n-vot2', 'VOTED', 'confirmed', 'Sim'));
  const ids = new Set(nodes.map(n => n.id));
  return { nodes, links: links.filter(l => ids.has(l.source) && ids.has(l.target)) };
}

/** Where a demo claim stands (claimsService.claimState, kept apart: the services import the mock core). */
function demoClaimState(claim: Claim): 'contradicted' | 'fulfilled' | 'review' | 'none' {
  const live = claim.findings.filter(f => f.status !== 'rejected');
  const own = (f: Claim['findings'][number]) => !f.actor || f.actor.scope === 'self';
  if (live.some(f => own(f) && f.label === 'CONTRADICTS' && f.status === 'confirmed')) return 'contradicted';
  if (live.some(f => own(f) && f.label === 'FULFILLS' && f.status === 'confirmed')) return 'fulfilled';
  return live.some(f => f.status === 'suggested') ? 'review' : 'none';
}

/** Home's card of a demo investigation. */
function investigationStatus(investigation: Investigation): InvestigationStatus {
  const docs = state.documents.filter(d => docInvestigation(d) === investigation.id);
  const sources = state.sources.filter(s => s.investigationId === investigation.id);
  const claims = investigation.id === 'inv-demo' ? state.claims : [];
  const tally = (state: string, label: string) => ({
    confirmed: claims.filter(c => demoClaimState(c) === state).length,
    suggested: claims.filter(c => demoClaimState(c) === 'review' && c.findings.some(f => f.status === 'suggested' && f.label === label)).length,
  });
  const findings = claims.flatMap(c => c.findings.filter(f => f.status !== 'rejected' && (!f.actor || f.actor.scope === 'self')).map(f => ({ claim: c, finding: f })));
  const latest = (['confirmed', 'suggested'] as const)
    .map(status => findings.filter(({ finding }) => finding.status === status).sort((a, b) => b.finding.date.localeCompare(a.finding.date))[0])
    .find(Boolean);
  const involved = (state.scandals[investigation.id]?.scandals ?? []).flatMap(scandal => scandal.people.map(person => ({ scandal, person })));
  const scandal = (['confirmed', 'suggested'] as const).map(status => involved.find(({ person }) => person.status === status)).find(Boolean);
  const schedule = state.schedules[investigation.id] ?? { paused: false, every: { feeds: null, news: null, videos: null, pages: null, gazettes: null, official: null }, window: null, days: [] };
  // Hours only (the demo leaves days out): open inside the window, overnight windows too
  const hhmm = new Date().toTimeString().slice(0, 5);
  const inWindow = !schedule.window || (schedule.window.start <= schedule.window.end ? hhmm >= schedule.window.start && hhmm < schedule.window.end : hhmm >= schedule.window.start || hhmm < schedule.window.end);
  const today = new Date().toISOString().slice(0, 10);
  return {
    id: investigation.id,
    name: investigation.name,
    kind: investigation.kind,
    targets: state.targets.filter(t => t.investigationId === investigation.id).map(t => t.name),
    sources: sources.filter(s => s.status !== 'suggested' && s.status !== 'rejected').length,
    failing: sources.filter(s => s.lastError && s.status !== 'paused').length,
    documents: docs.length,
    unread: docs.filter(d => FEED_KINDS.includes(d.kind) && !d.read).length,
    lastUpdate: docs.map(d => d.fetchedAt).sort().pop() ?? null,
    indexing: investigation.id === 'inv-prefeita' ? 4 : 0,
    queued: sources.reduce((sum, s) => sum + s.queued, 0),
    schedule: {
      schedule,
      open: !schedule.paused && inWindow,
      nextOpen: !schedule.paused && !inWindow && schedule.window ? `${today}T${schedule.window.start}:00` : null,
      nextRunAt: sources.map(s => s.nextRunAt).sort()[0] ?? null,
    },
    contradictions: tally('contradicted', 'CONTRADICTS'),
    fulfilled: tally('fulfilled', 'FULFILLS'),
    insights:
      claims.flatMap(c => c.findings).filter(f => f.status === 'suggested').length +
      sources.filter(s => s.status === 'suggested').length +
      involved.filter(({ person }) => person.status === 'suggested').length,
    latest: scandal && (!latest || scandal.person.status === 'confirmed')
      ? {
          kind: 'scandal',
          text: scandal.scandal.title,
          person: scandal.person.name,
          date: scandal.scandal.date ?? now(),
          status: scandal.person.status as 'confirmed' | 'suggested',
          documentId: scandal.scandal.documentId,
          sessionId: null,
        }
      : latest
      ? {
          kind: latest.finding.label === 'CONTRADICTS' ? 'contradiction' : 'fulfillment',
          text: latest.claim.text,
          person: state.targets.find(t => t.id === latest.claim.targetId)?.name ?? null,
          date: latest.finding.date,
          status: latest.finding.status as 'confirmed' | 'suggested',
          documentId: latest.claim.source.documentId,
          sessionId: null,
        }
      : null,
    diskBytes: docs.reduce((sum, d) => sum + d.text.length * 6, 0) + 3_500_000,
  };
}

/** Home's scoreboard in the demo: the claims' states, the scandals, and the fictional live checks of the demo session. */
function scoreboard(): PersonScore[] {
  const scores = state.targets
    .filter(t => t.kind === 'person' && state.investigations.find(i => i.id === t.investigationId)?.kind === 'politics')
    .map(t => {
      const claims = state.claims.filter(c => c.targetId === t.id);
      const involved = Object.values(state.scandals).flatMap(s => s.scandals.flatMap(scandal => scandal.people)).filter(p => p.targetId === t.id);
      // The demo session: one statement checked false, one true (fictional)
      const checks = t.id === 'target-marina' ? 1 : 0;
      return {
        targetId: t.id,
        name: t.name,
        investigationId: t.investigationId,
        investigationName: state.investigations.find(i => i.id === t.investigationId)?.name ?? '',
        contradictions: claims.filter(c => demoClaimState(c) === 'contradicted').length,
        falseChecks: checks,
        fulfilled: claims.filter(c => demoClaimState(c) === 'fulfilled').length,
        trueChecks: checks,
        scandals: involved.filter(p => p.status === 'confirmed').length,
        sanctioned: Object.values(state.scandals).flatMap(s => s.sanctioned).filter(s => s.targetId === t.id).length,
        toReview: claims.flatMap(c => c.findings).filter(f => f.status === 'suggested').length + involved.filter(p => p.status === 'suggested').length,
      };
    });
  return scores.sort((a, b) => b.contradictions + b.falseChecks - (a.contradictions + a.falseChecks) || a.name.localeCompare(b.name));
}

/** A comparison in the demo, from the demo documents (fictional people and outlets). */
function demoComparison(ids: string[]): Comparison {
  const sides = ids.map(id => {
    const investigation = state.investigations.find(i => i.id === id)!;
    return {
      id,
      name: investigation.name,
      kind: investigation.kind,
      documents: state.documents.filter(d => docInvestigation(d) === id).length,
      targets: state.targets.filter(t => t.investigationId === id).map(t => t.name),
    };
  });
  const passage = (d: DemoDocument, scores: number[]) => ({
    investigationId: docInvestigation(d),
    documentId: d.id,
    chunkId: `chunk-${d.id}`,
    title: d.title,
    site: d.site,
    date: docDate(d),
    scores,
  });
  const own = (id: string) => state.documents.filter(d => docInvestigation(d) === id && FEED_KINDS.includes(d.kind)).sort((a, b) => (b.relevance ?? 0) - (a.relevance ?? 0));
  // The first news of each side bridges them; the rest is where they diverge
  const bridges = ids.flatMap(id => own(id).slice(0, 1).map(d => passage(d, ids.map(other => (other === id ? 0.62 : 0.58)))));
  const divergent = ids.flatMap(id => own(id).slice(1, 4).map(d => passage(d, ids.map(other => (other === id ? 0.7 : 0.2)))));
  const today = new Date();
  const monday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - ((today.getDay() + 6) % 7));
  const timeline = Array.from({ length: 26 }, (_, i) => {
    const start = new Date(monday.getTime() - (25 - i) * 7 * 86400000);
    const end = new Date(start.getTime() + 7 * 86400000);
    return {
      week: start.toISOString().slice(0, 10),
      counts: ids.map(id => state.documents.filter(d => docInvestigation(d) === id && new Date(docDate(d)) >= start && new Date(docDate(d)) < end).length),
    };
  });
  const both = ids.includes('inv-demo') && ids.includes('inv-prefeita');
  return {
    sides,
    sameDocuments: [],
    shared: both
      ? [
          {
            key: 'cnpj:11222333000181',
            kind: 'organization',
            label: 'Gráfica Exemplo Ltda',
            ties: [
              { investigationId: 'inv-demo', who: 'Marina Exemplo', label: 'SPENT_WITH', date: '2026-08' },
              { investigationId: 'inv-prefeita', who: 'Prefeitura de Cidade Exemplo', label: 'CONTRACTED', date: '2026-08-20' },
            ],
          },
        ]
      : [],
    votes: { both: 0, agreed: 0, disagreements: [] },
    topics: ids.includes('inv-demo') ? state.claims.map(c => ({ sides: [{ investigationId: 'inv-demo', topic: c.topic, claims: 1, contradicted: demoClaimState(c) === 'contradicted' ? 1 : 0, fulfilled: demoClaimState(c) === 'fulfilled' ? 1 : 0 }] })) : [],
    bridges,
    divergent,
    timeline,
    notice: null,
  };
}

function demoCompareSummary(ids: string[]): CompareSummary {
  const { bridges, divergent, sides } = demoComparison(ids);
  const cited = [...bridges, ...divergent].slice(0, 4);
  const [a, b] = sides;
  return {
    text: `## Onde se conectam\n\n${a.name} e ${b.name} se encontram no que acontece em Cidade Exemplo [1] [2].\n\n## Onde divergem\n\nO restante do que cada lado coletou trata de assuntos próprios [3] [4].\n\n## O que ainda falta saber\n\nHá pouco material em comum: vale acompanhar mais fontes.\n\n*Demonstração: pessoas e veículos fictícios.*`,
    citations: cited.map((p, i) => {
      const d = state.documents.find(doc => doc.id === p.documentId)!;
      return { index: i + 1, chunkId: p.chunkId, documentId: d.id, title: d.title, url: d.url, site: d.site, kind: d.kind, docDate: docDate(d), startTime: null, excerpt: d.text.slice(0, 300) };
    }),
  };
}

const plain = (text: string) => text.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().trim();

export const handlers: Record<string, Handler> = {
  ...liveHandlers(state.documents, {
    known: () =>
      state.targets
        .filter(t => t.kind === 'person')
        .map(t => ({ targetId: t.id, name: t.name, aliases: t.aliases, investigationId: t.investigationId, investigationName: state.investigations.find(i => i.id === t.investigationId)?.name ?? '' })),
    // As the core does: someone followed under that name, else an investigation of their own
    follow: name => {
      const known = state.targets.find(t => t.kind === 'person' && [t.name, ...t.aliases].some(n => plain(n) === plain(name)));
      if (known) return known.id;
      const investigation = handlers.create_investigation({ name: name.trim() }) as Investigation;
      return handlers.save_target({ target: { id: null, investigationId: investigation.id, kind: 'person', name: name.trim(), aliases: [] } }) as string;
    },
  }),

  get_ai_settings: () => ({ ...state.ai, keys: state.keys }),
  save_ai_settings: ({ settings }) => {
    state.ai = settings;
  },
  save_api_key: ({ provider, key }) => {
    state.keys = state.keys.filter(k => k !== provider).concat(key ? [provider] : []);
  },
  list_ollama_models: () => OLLAMA_MODELS,
  builtin_ai_list_models: ({ kind }) => state.builtin.filter(m => m.kind === (kind ?? 'summary')),
  builtin_ai_get_recommended_model: () => 'qwen3.5:4b',
  builtin_ai_computer_profile: () => (mockBasic() ? { ramGb: 4, gpu: false, basic: true } : { ramGb: 16, gpu: true, basic: false }),
  list_provider_models: ({ provider, key }) => {
    if (!key && !state.keys.includes(provider) && !['custom-openai', 'claude-code'].includes(provider)) throw new Error(`${provider} refused the key (HTTP 401)`);
    return PROVIDER_MODELS[provider] ?? [];
  },
  builtin_ai_download_model: ({ modelName }) => simulateDownload(modelName),
  builtin_ai_cancel_download: ({ modelName }) => {
    downloads.get(modelName)?.cancel();
    progressEvent({ model: modelName, progress: 0, status: 'cancelled' });
  },
  builtin_ai_delete_model: ({ modelName }) => {
    const model = state.builtin.find(m => m.name === modelName);
    if (model) model.status = { type: 'not_downloaded' };
  },
  test_ai_model: () => 'ok',
  get_collect_settings: () => ({ analysis: state.analysis, fullProposals: state.fullProposals, newsImages: state.newsImages }),
  save_collect_settings: ({ settings }) => {
    state.analysis = settings.analysis;
    state.fullProposals = settings.fullProposals;
    state.newsImages = settings.newsImages;
  },
  dashboard: () => state.investigations.map(investigationStatus),

  list_investigations: () => state.investigations.map(withCounts),
  create_investigation: ({ name, description, kind }) => {
    const investigation: Investigation = { id: id('inv'), name, description: description ?? '', kind: kind ?? 'politics', createdAt: now(), documentCount: 0, sourceCount: 0 };
    state.investigations.push(investigation);
    return investigation;
  },
  update_investigation: ({ id, name, description }) => {
    const investigation = state.investigations.find(i => i.id === id)!;
    Object.assign(investigation, { name, description });
    return withCounts(investigation);
  },
  set_investigation_kind: ({ id, kind }) => {
    const investigation = state.investigations.find(i => i.id === id)!;
    investigation.kind = kind;
    state.sources = state.sources.filter(s => !(s.investigationId === id && s.status === 'suggested'));
    return withCounts(investigation);
  },
  // As the core does: what it collected goes with it
  delete_investigation: ({ id }) => {
    state.documents = state.documents.filter(d => docInvestigation(d) !== id);
    state.investigations = state.investigations.filter(i => i.id !== id);
    state.targets = state.targets.filter(t => t.investigationId !== id);
    state.sources = state.sources.filter(s => s.investigationId !== id);
  },

  list_targets: ({ investigationId }) => state.targets.filter(t => t.investigationId === investigationId),
  save_target: ({ target }) => {
    if (target.id) {
      Object.assign(state.targets.find(t => t.id === target.id)!, target);
      return target.id;
    }
    const created = { ...target, id: id('target'), official: [], createdAt: now() };
    state.targets.push(created);
    // As the core does: no gazette search in an interest
    const interest = state.investigations.find(i => i.id === target.investigationId)?.kind === 'interest';
    const searches: SourceKind[] = interest ? ['news_search', 'youtube_search'] : ['news_search', 'youtube_search', 'gazette_search'];
    for (const kind of searches) {
      state.sources.push({
        ...SOURCES.find(s => s.id === 'src-news')!,
        id: id('src'),
        investigationId: target.investigationId,
        targetId: created.id,
        kind,
        url: `"${target.name}"`,
        title: target.name,
        lastRunAt: null,
        documentCount: 0,
        queued: 0,
      });
    }
    return created.id;
  },
  find_official_matches: ({ targetId }) => {
    const target = state.targets.find(t => t.id === targetId);
    return target?.name.includes('Marina') ? OFFICIAL_MATCHES : [];
  },
  link_official: ({ targetId, matches }) => {
    const target = state.targets.find(t => t.id === targetId)!;
    const added = (matches as OfficialMatch[]).filter(m => !target.official.some(o => o.id === m.id));
    target.official = [...target.official, ...added];
    for (const match of added) {
      state.sources.push({
        ...SOURCES.find(s => s.id === 'src-news')!,
        id: id('src'),
        investigationId: target.investigationId,
        targetId,
        kind: match.kind,
        url: match.id,
        title: `${match.label} · ${match.detail}`,
        everyMinutes: 1440,
        lastRunAt: null,
        documentCount: 0,
        queued: 0,
      });
    }
  },
  delete_target: ({ id }) => {
    state.targets = state.targets.filter(t => t.id !== id);
    state.sources = state.sources.filter(s => s.targetId !== id);
  },

  list_sources: ({ investigationId }) => state.sources.filter(s => s.investigationId === investigationId),
  add_source: ({ investigationId, kind, url, title }) => {
    const followLinks = kind === 'rss' || kind === 'page';
    const created = { ...SOURCES.find(s => s.id === 'src-rss')!, id: id('src'), investigationId, kind, url, title, followLinks, lastRunAt: null, lastError: null, documentCount: 0, queued: 0 };
    state.sources.push(created);
    return created.id;
  },
  update_source: ({ id, status, everyMinutes, followLinks }) => {
    const source = state.sources.find(s => s.id === id)!;
    if (status) source.status = status;
    if (everyMinutes) source.everyMinutes = everyMinutes;
    if (followLinks != null) source.followLinks = followLinks;
  },
  get_discovery: () => ({ limits: state.discovery, usage: { pages: 37, aiCalls: 4 } }),
  save_discovery: ({ limits }) => {
    state.discovery = limits;
  },
  get_collect_schedule: ({ investigationId }) => {
    const schedule = state.schedules[investigationId] ?? { paused: false, every: { feeds: null, news: null, videos: null, pages: null, gazettes: null, official: null }, window: null, days: [] };
    return { schedule, open: !schedule.paused };
  },
  save_collect_schedule: ({ investigationId, schedule }) => {
    state.schedules[investigationId] = schedule;
  },
  approve_source: ({ id }) => {
    const source = state.sources.find(s => s.id === id);
    if (source) source.status = 'active';
  },
  reject_source: ({ id }) => {
    const source = state.sources.find(s => s.id === id);
    if (source) source.status = 'rejected';
  },
  suggest_sources: ({ investigationId }) => {
    state.sources.unshift({
      ...SOURCES.find(s => s.id === 'src-sug-creches')!,
      id: id('src'),
      investigationId,
      url: 'Marina Exemplo cesta básica isenção votação',
      note: 'Procura a votação da isenção da cesta básica que ela prometeu apoiar.',
    });
    return 1;
  },
  get_websearch_settings: () => ({ ...state.webSearch, key: '' }),
  save_websearch_settings: ({ settings }) => {
    state.webSearch = { ...settings, hasKey: state.webSearch.hasKey || !!settings.key };
  },
  test_websearch: () => 20,
  delete_source: ({ id }) => {
    state.sources = state.sources.filter(s => s.id !== id);
  },
  run_source_now: ({ id }) => {
    const source = state.sources.find(s => s.id === id);
    if (source) source.lastRunAt = now();
  },
  // The demo is collecting for the AI subject
  collect_status: () => ({
    running: true,
    current: 'Fábricas de Cidade Exemplo usam IA para prever falhas',
    queued: 9,
    collected: 0,
    analyzing: false,
    investigationId: 'inv-ia',
    runs: [{ investigationId: 'inv-ia', done: 3, total: 12 }],
  }),

  list_claims: ({ investigationId }) => (investigationId === 'inv-demo' ? state.claims : []),
  set_finding_status: ({ id, status }) => {
    for (const claim of state.claims) for (const finding of claim.findings) if (finding.id === id) finding.status = status;
    for (const scandal of Object.values(state.scandals).flatMap(s => s.scandals)) for (const person of scandal.people) if (person.relationId === id) person.status = status;
  },
  list_scandals: ({ investigationId }) => {
    const found = state.scandals[investigationId] ?? { scandals: [], sanctioned: [] };
    return { ...found, scandals: found.scandals.filter(s => s.people.some(p => p.status !== 'rejected')) };
  },
  mark_scandal: ({ documentId, title, date, targetIds }) => {
    const document = state.documents.find(d => d.id === documentId)!;
    const investigationId = docInvestigation(document);
    const scandal = {
      id: id('scandal'),
      investigationId,
      title,
      date,
      documentId,
      documentTitle: document.title,
      people: (targetIds as string[]).map(targetId => ({
        relationId: id('rel'),
        targetId,
        name: state.targets.find(t => t.id === targetId)?.name ?? '',
        status: 'confirmed' as const,
        origin: 'user' as const,
        rationale: null,
      })),
    };
    (state.scandals[investigationId] ??= { scandals: [], sanctioned: [] }).scandals.unshift(scandal);
    return scandal.id;
  },
  delete_scandal: ({ id }) => {
    for (const found of Object.values(state.scandals)) found.scandals = found.scandals.filter(s => s.id !== id);
  },
  scoreboard: () => scoreboard(),
  delete_claim: ({ id }) => {
    state.claims = state.claims.filter(c => c.id !== id);
  },
  analysis_status: () => ({ pending: 1, analyzed: 3, failed: 0 }),

  investigation_graph: ({ investigationIds, options }) => {
    const graph = (investigationIds as string[]).includes('inv-demo') ? demoGraph(!!options?.allVotes, !!options?.suggested) : { nodes: [], links: [] };
    // Next to the city hall's investigation, its city hall and the supplier it contracted are shared
    const shared = (key: string) => (investigationIds.includes('inv-prefeita') && ['n-prefeitura', 'n-grafica'].includes(key) ? ['inv-demo', 'inv-prefeita'] : ['inv-demo']);
    return { ...graph, nodes: graph.nodes.map(n => ({ ...n, investigations: investigationIds.length > 1 ? shared(n.key) : [] })) };
  },
  compare_investigations: ({ investigationIds }) => demoComparison(investigationIds),
  compare_summary: ({ investigationIds }) => demoCompareSummary(investigationIds),
  enrich_entity: async ({ entityId }) => {
    if (entityId !== 'n-grafica') throw new Error('Demo: only Gráfica Exemplo has public data');
    await new Promise(resolve => setTimeout(resolve, 800));
    state.graficaPublicData = {
      razao_social: 'GRAFICA EXEMPLO LTDA', situacao: 'ATIVA', cnae: 'Impressão de material para uso publicitário',
      municipio: 'SAO PAULO', uf: 'SP', abertura: '2019-03-11', socios: 1, receita_at: new Date().toISOString(),
      sanctions: [{ cadastro: 'CEIS', categoria: 'Impedimento de licitar e contratar', orgao: 'Prefeitura de Cidade Exemplo', inicio: '10/02/2025', fim: '10/02/2027', processo: '123/2024' }],
      sanctions_checked: new Date().toISOString().slice(0, 10),
    };
  },
  get_flowsint_settings: () => ({ ...state.flowsint, password: '' }),
  save_flowsint_settings: ({ settings }) => {
    state.flowsint = { ...settings, hasPassword: state.flowsint.hasPassword || !!settings.password };
  },
  test_flowsint: () => {
    if (!state.flowsint.endpoint) throw new Error('Demo: type any address');
  },
  create_flowsint_account: () => undefined,
  push_to_flowsint: () => {
    const base = state.flowsint.mode === 'local' ? LOCAL_FLOWSINT : 'https://flowsint.example';
    state.flowsintSync = { investigation: 'demo', sketch: 'demo', pushedAt: now(), url: `${base}/dashboard` };
    return { url: state.flowsintSync.url, nodes: 10, edges: 11, nodesCreated: 10, errors: [] };
  },
  flowsint_sync: () => state.flowsintSync,
  flowsint_local_status: () => ({
    engine: 'running',
    stack: state.flowsintLocal.installed ? (state.flowsintLocal.running ? 'running' : 'stopped') : 'absent',
    installed: state.flowsintLocal.installed,
    url: state.flowsintLocal.installed ? LOCAL_FLOWSINT : null,
    email: 'voga@example.com',
    password: state.flowsintLocal.installed ? 'demo-password' : null,
    engineApp: 'Docker Desktop',
    busy: state.flowsintLocal.busy,
    progress: null,
    version: '1.2.13',
  }),
  flowsint_local_install: () => simulateFlowsint(['pull', 'start', 'account']),
  flowsint_local_start: () => simulateFlowsint(['start']),
  flowsint_local_stop: () => {
    state.flowsintLocal.running = false;
  },
  flowsint_local_remove: () => {
    state.flowsintLocal = { installed: false, running: false, busy: false };
  },
  open_docker: () => undefined,

  list_documents: ({ investigationIds, kind, limit, offset }) => {
    const all = state.documents
      .filter(d => investigationIds.includes(docInvestigation(d)) && (!kind || d.kind === kind))
      .sort((a, b) => docDate(b).localeCompare(docDate(a)));
    const start = offset ?? 0;
    return { items: all.slice(start, start + (limit ?? 50)).map(summary), total: all.length };
  },
  get_document: ({ id }) => {
    const document = state.documents.find(d => d.id === id)!;
    return { ...document, investigationId: docInvestigation(document), starred: !!document.starred, feedback: document.feedback ?? 0 };
  },

  list_feed: ({ query }) => {
    const kindOf = (d: DemoDocument) => state.investigations.find(i => i.id === docInvestigation(d))?.kind;
    const all = state.documents
      .filter(d => FEED_KINDS.includes(d.kind) && (!query.investigationIds || query.investigationIds.includes(docInvestigation(d))))
      .filter(d => !query.investigationKind || kindOf(d) === query.investigationKind)
      .filter(d => (!query.since || docDate(d) >= query.since) && (!query.unreadOnly || !d.read) && (!query.starredOnly || d.starred))
      .sort((a, b) => (query.sort === 'relevance' ? (b.relevance ?? -2) - (a.relevance ?? -2) : 0) || docDate(b).localeCompare(docDate(a)));
    const start = query.offset ?? 0;
    return {
      items: all.slice(start, start + (query.limit ?? 50)).map(d => ({
        ...summary(d),
        investigationId: docInvestigation(d),
        investigationName: state.investigations.find(i => i.id === docInvestigation(d))?.name ?? '',
        excerpt: d.text.slice(0, 600),
        relevance: d.relevance ?? null,
        read: !!d.read,
        starred: !!d.starred,
        feedback: d.feedback ?? 0,
        imageUrl: state.newsImages === 'image' ? null : (d.imageUrl ?? null),
        imagePath: null,
      })),
      total: all.length,
    };
  },
  mark_read: ({ ids }) => {
    state.documents.filter(d => ids.includes(d.id)).forEach(d => (d.read = true));
  },
  mark_all_read: ({ investigationIds, investigationKind }) => {
    state.documents
      .filter(d => !investigationIds || investigationIds.includes(docInvestigation(d)))
      .filter(d => !investigationKind || state.investigations.find(i => i.id === docInvestigation(d))?.kind === investigationKind)
      .forEach(d => (d.read = true));
  },
  set_document_flags: ({ id, starred, feedback }) => {
    const document = state.documents.find(d => d.id === id)!;
    if (starred != null) document.starred = starred;
    if (feedback != null) document.feedback = feedback;
  },
  delete_document: ({ id }) => {
    state.documents = state.documents.filter(d => d.id !== id);
  },

  search_documents: ({ investigationIds, query }) => {
    const words = String(query).toLowerCase().split(/\s+/).filter(w => w.length > 2);
    const results = state.documents
      .filter(d => investigationIds.includes(docInvestigation(d)))
      .flatMap(d => {
        const passages = d.segments ? d.segments.map(s => ({ text: s.text, start: s.start, end: s.end })) : [{ text: d.text, start: null, end: null }];
        return passages
          .filter(p => words.some(w => `${d.title} ${p.text}`.toLowerCase().includes(w)))
          .map(p => ({
            chunkId: `chunk-${d.id}-${p.start ?? 0}`,
            documentId: d.id,
            title: d.title,
            url: d.url,
            kind: d.kind,
            site: d.site,
            docDate: docDate(d),
            text: p.text,
            startTime: p.start,
            endTime: p.end,
            score: 1,
            sources: ['lexical'],
          }));
      });
    return { results, vectorError: null };
  },
  ask_question: ({ request }) => {
    const answer = { ...demoAnswer(request.question), historyId: id('ask') };
    state.asked.unshift({ id: answer.historyId!, investigationId: request.investigationIds.join(','), question: request.question, createdAt: now(), answer });
    return answer;
  },
  list_ask_history: ({ investigationIds }) =>
    state.asked.filter(a => a.investigationId === investigationIds.join(',')).map(({ id, question, createdAt }) => ({ id, question, createdAt })),
  get_ask_answer: ({ id }) => state.asked.find(a => a.id === id)?.answer ?? null,
};

/** What an unhandled command returns: empty lists for list_*, null for the rest. */
export function fallback(cmd: string): unknown {
  if (/^list_|_models$/.test(cmd)) return [];
  return null;
}
