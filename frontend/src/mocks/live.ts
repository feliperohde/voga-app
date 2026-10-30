/**
 * Mock mode: a live session acted out with the demo's fictional people. Starting one plays a
 * short debate: a moderator asking questions (her voice asked about), Marina Exemplo (her voice
 * recognized) and Otávio Fictício (named, not followed), with an affirmation or a denial of each
 * verdict against the demo records; checked as said, or when "Checar" is clicked. A debate plays
 * the same script with its people: the first voice is Marina's when she debates (else the first
 * person's), and the third voice asks who it is among the others.
 */

import { emit } from '@tauri-apps/api/event';
import type { BuiltinModel } from '@/services/builtinService';
import type { ClaimKind, Evidence, KnownPerson, LiveCheck, LiveLine, LiveSessionSummary, LiveState, LiveVoice, SessionPerson, Verdict } from '@/services/liveService';
import { DemoDocument, DOCUMENTS } from './data';

export const LIVE_MODELS: BuiltinModel[] = [
  { kind: 'transcription', name: 'whisper:large-v3-turbo', display_name: 'Whisper Large v3 Turbo', size_mb: 547, status: { type: 'available' },
    description: 'Recommended: the best Portuguese transcription, fast with a GPU (Apple Silicon).' },
  { kind: 'transcription', name: 'whisper:small', display_name: 'Whisper Small', size_mb: 181, status: { type: 'not_downloaded' },
    description: 'For computers without a GPU: keeps up on the processor, with more mistakes.' },
  { kind: 'voice', name: 'voice:wespeaker', display_name: 'WeSpeaker ResNet34', size_mb: 25, status: { type: 'available' },
    description: 'Tells voices apart, to know who said each line.' },
];

const MARINA = 'target-marina';
const IDLE: LiveState = { sessionId: null, investigationId: null, phase: 'idle', startedAt: null, behind: 0, voices: false, checking: false };

/** The debate: who speaks (1: Marina, 2: the moderator, 3: Otávio), what, and the claim it brings. */
const SCRIPT: {
  voice: 1 | 2 | 3;
  text: string;
  check?: { kind: ClaimKind; claim: string; quote: string; verdict: Verdict; evidence?: string; rationale?: string };
}[] = [
  { voice: 2, text: 'Boa noite. Primeira pergunta, para a deputada Marina Exemplo: como a senhora votou na reforma dos impostos?' },
  {
    voice: 1,
    text: 'Boa noite. Eu votei a favor do PLP 99 de 2025, que cria o imposto sobre bens e serviços.',
    check: { kind: 'affirmation', claim: 'Marina Exemplo afirma ter votado a favor do PLP 99/2025.', quote: 'Eu votei a favor do PLP 99 de 2025', verdict: 'false',
      evidence: 'doc-votacao-oficial', rationale: 'No registro da Câmara, Marina Exemplo votou "Não" no texto-base do PLP 99/2025.' },
  },
  {
    voice: 1,
    text: 'E apresentei o projeto Creche Perto de Casa, o PL 1234 de 2025.',
    check: { kind: 'affirmation', claim: 'Marina Exemplo afirma ter apresentado o PL 1234/2025, Creche Perto de Casa.', quote: 'apresentei o projeto Creche Perto de Casa, o PL 1234 de 2025',
      verdict: 'true', evidence: 'doc-projeto', rationale: 'O PL 1234/2025, que cria o programa Creche Perto de Casa, é de autoria de Marina Exemplo.' },
  },
  { voice: 2, text: 'Candidato Otávio Fictício, o senhor concorda com a deputada?' },
  { voice: 3, text: 'Não concordo. A emenda dela prometeu três creches que nunca saíram do papel.' },
  { voice: 2, text: 'Deputada, a senhora quer responder?' },
  {
    voice: 1,
    text: 'Isso não é verdade: eu nunca falei em três creches, a minha emenda levou quatro milhões para duas creches na zona norte.',
    check: { kind: 'denial', claim: 'Marina Exemplo nega ter prometido três creches e diz que a emenda levou R$ 4 milhões para duas.', quote: 'eu nunca falei em três creches, a minha emenda levou quatro milhões para duas creches',
      verdict: 'true', evidence: 'doc-creche', rationale: 'A notícia registra a emenda de R$ 4 milhões para duas creches na zona norte.' },
  },
  {
    voice: 1,
    text: 'E na nossa cidade o desemprego caiu trinta por cento no último ano.',
    check: { kind: 'affirmation', claim: 'Marina Exemplo afirma que o desemprego na cidade caiu 30% no último ano.', quote: 'o desemprego caiu trinta por cento no último ano', verdict: 'unverified' },
  },
  { voice: 2, text: 'Obrigada. O tempo do bloco terminou.' },
];

const SECONDS_PER_LINE = 4;

function evidenceOf(documentId: string): Evidence[] {
  const doc = DOCUMENTS.find(d => d.id === documentId);
  if (!doc) return [];
  return [{ documentId: doc.id, chunkId: `${doc.id}-0`, title: doc.title, kind: doc.kind, date: (doc.publishedAt ?? doc.fetchedAt).slice(0, 10), url: doc.url, site: doc.site,
    excerpt: doc.text.slice(0, 600), startTime: null, actor: doc.kind === 'vote' || doc.kind === 'bill' ? { targetId: MARINA, name: 'Marina Exemplo', party: 'PDE', scope: 'self' } : null }];
}

const say = (event: string, payload: unknown) => emit(event, payload).catch(() => {});

/** The demo's people: everyone followed, and following someone new (an investigation of their own). */
export interface DemoPeople {
  known: () => KnownPerson[];
  follow: (name: string) => string;
}

/** The handlers of the live commands; `documents` is the demo's list, a session ended joins it. */
export function liveHandlers(documents: DemoDocument[], people: DemoPeople) {
  const live = {
    state: IDLE,
    consent: true,
    model: 'whisper:large-v3-turbo' as string | null,
    transcriptionProvider: 'builtin',
    transcriptionRemoteModel: '',
    sessions: [] as LiveSessionSummary[],
    lines: [] as LiveLine[],
    voices: [] as (LiveVoice & { sessionId: string })[],
    checks: [] as LiveCheck[],
    /** Each debate's people, in order. */
    debaters: new Map<string, SessionPerson[]>(),
    /** What each check comes out as, when it is checked. */
    outcomes: new Map<string, LiveCheck>(),
    timers: [] as ReturnType<typeof setTimeout>[],
  };
  const settle = (id: string) => {
    const check = live.checks.find(c => c.id === id);
    const done = live.outcomes.get(id);
    if (!check || !done) return;
    check.verdict = 'checking';
    say('live-check', check);
    // As in the app, a check goes on after the session stops
    setTimeout(() => {
      live.checks = live.checks.map(c => (c.id === id ? { ...done, status: c.status } : c));
      say('live-check', live.checks.find(c => c.id === id));
    }, 2500);
  };
  const setState = (state: LiveState) => {
    live.state = state;
    say('live-state', state);
  };
  const counted = (session: LiveSessionSummary): LiveSessionSummary => {
    const checks = live.checks.filter(c => c.sessionId === session.id);
    const count = (verdict: Verdict) => checks.filter(c => c.verdict === verdict).length;
    return { ...session, lines: live.lines.filter(l => l.sessionId === session.id).length,
      trueCount: count('true'), falseCount: count('false'), partlyCount: count('partly'), unverifiedCount: count('unverified') };
  };
  const voiceView = (voice: LiveVoice & { sessionId: string }) => {
    const { sessionId, ...view } = voice;
    return { sessionId, view };
  };
  const announce = (voice: LiveVoice & { sessionId: string }) => {
    live.voices.push(voice);
    const { sessionId, view } = voiceView(voice);
    say('live-voice', { sessionId, voice: view });
  };

  const personOf = (known: KnownPerson): SessionPerson => ({ id: known.targetId, name: known.name, investigationId: known.investigationId });
  const peopleOf = (session: LiveSessionSummary): SessionPerson[] =>
    session.investigationId ? people.known().filter(k => k.investigationId === session.investigationId).map(personOf) : (live.debaters.get(session.id) ?? []);
  const nameOf = (targetId: string) => people.known().find(k => k.targetId === targetId)?.name ?? '?';

  function act(sessionId: string, debaters?: SessionPerson[]) {
    // The first voice: Marina, or a debate's first person; the third, asked about in a debate
    const speaker = debaters ? (debaters.find(p => p.id === MARINA) ?? debaters[0]).id : MARINA;
    const others = (debaters ?? []).filter(p => p.id !== speaker);
    SCRIPT.forEach((line, i) => {
      const start = 2 + i * SECONDS_PER_LINE;
      live.timers.push(
        setTimeout(() => {
          const view: LiveLine = { id: live.lines.length + 1, sessionId, start, end: start + SECONDS_PER_LINE - 0.5, text: line.text, voiceId: `voice-${line.voice}` };
          live.lines.push(view);
          say('live-transcript', view);
          if (i === 1) {
            announce({ sessionId, voiceId: 'voice-1', number: 1, targetId: speaker, label: null, role: null, resolved: true, recognized: true, seconds: 6, heard: [], suggestions: [] });
            announce({ sessionId, voiceId: 'voice-2', number: 2, targetId: null, label: null, role: null, resolved: false, recognized: false, seconds: 5, heard: [],
              suggestions: [{ targetId: speaker, name: nameOf(speaker), similarity: 0.18, taken: true }, ...others.map(p => ({ targetId: p.id, name: p.name, similarity: null, taken: false }))] });
          }
          if (line.voice === 3 && !live.voices.some(v => v.sessionId === sessionId && v.voiceId === 'voice-3')) {
            announce(
              debaters
                ? { sessionId, voiceId: 'voice-3', number: 3, targetId: null, label: null, role: null, resolved: false, recognized: false, seconds: 5, heard: [],
                    suggestions: [...others.map(p => ({ targetId: p.id, name: p.name, similarity: null, taken: false })), { targetId: speaker, name: nameOf(speaker), similarity: 0.12, taken: true }] }
                : // Named by the user when asked: not followed, so not checked
                  { sessionId, voiceId: 'voice-3', number: 3, targetId: null, label: 'Otávio Fictício', role: null, resolved: true, recognized: false, seconds: 5, heard: [], suggestions: [] },
            );
          }
          if (!line.check) return;
          const session = live.sessions.find(s => s.id === sessionId);
          const check: LiveCheck = { id: `check-${sessionId}-${i}`, sessionId, targetId: speaker, start: view.start, end: view.end, quote: line.check.quote, claim: line.check.claim, topic: '',
            kind: line.check.kind, verdict: 'detected', confidence: null, rationale: null, evidence: [], webSearched: false, status: 'suggested', error: null };
          live.outcomes.set(check.id, { ...check, verdict: line.check.verdict, rationale: line.check.rationale ?? null,
            confidence: line.check.verdict === 'unverified' ? null : 0.86, evidence: line.check.evidence ? evidenceOf(line.check.evidence) : [] });
          live.checks.push(check);
          say('live-check', check);
          if (session?.autoCheck) settle(check.id);
        }, (start + SECONDS_PER_LINE) * 1000 - 2000),
      );
    });
    const level = setInterval(() => say('live-level', { level: 0.35 + Math.random() * 0.5 }), 150);
    live.timers.push(level as unknown as ReturnType<typeof setTimeout>);
  }

  function stop() {
    live.timers.forEach(timer => {
      clearTimeout(timer);
      clearInterval(timer);
    });
    live.timers = [];
    const sessionId = live.state.sessionId;
    setState({ ...live.state, phase: 'finishing' });
    setTimeout(() => {
      const session = live.sessions.find(s => s.id === sessionId);
      if (session) {
        const lines = live.lines.filter(l => l.sessionId === session.id);
        const name = (voiceId: string | null) => {
          const voice = live.voices.find(v => v.sessionId === session.id && v.voiceId === voiceId);
          return voice?.role === 'interviewer' ? 'Entrevistador' : voice?.targetId ? nameOf(voice.targetId) : (voice?.label ?? `Voz ${voice?.number ?? '?'}`);
        };
        const segments = lines.map(l => ({ start: l.start, end: l.end, text: `${name(l.voiceId)}: ${l.text}` }));
        const documentId = `doc-${session.id}`;
        documents.unshift({ id: documentId, sourceId: null, url: `voga://live/${session.id}`, kind: 'live', title: session.title, author: session.targetName, site: 'Ao vivo',
          publishedAt: session.startedAt, fetchedAt: new Date().toISOString(), note: null, indexError: null, text: segments.map(s => s.text).join('\n'), segments });
        Object.assign(session, { status: 'done', endedAt: new Date().toISOString(), documentId });
      }
      setState(IDLE);
    }, 1200);
  }

  const handlers: Record<string, (args: Record<string, any>) => unknown> = {
    live_status: () => ({ state: live.state, transcriptionModel: live.model, transcriptionProvider: live.transcriptionProvider,
      transcriptionRemoteModel: live.transcriptionRemoteModel, voiceModel: true, voiceConsent: live.consent, systemAudio: true,
      microphones: [{ name: 'Microfone do MacBook', isDefault: true }, { name: 'Microfone USB', isDefault: false }], chatModel: true }),
    live_save_settings: ({ settings }) => {
      live.consent = settings.voiceConsent;
      if (settings.transcriptionProvider != null) live.transcriptionProvider = settings.transcriptionProvider;
      if (settings.transcriptionRemoteModel != null) live.transcriptionRemoteModel = settings.transcriptionRemoteModel;
      live.model = settings.transcriptionModel || 'whisper:large-v3-turbo';
    },
    live_start: ({ investigationId, targetId, title, autoCheck }) => {
      if (live.state.phase !== 'idle') throw new Error('A live session is already running');
      const session: LiveSessionSummary = { id: `live-${Date.now()}`, investigationId, targetId, targetName: 'Marina Exemplo', title, notes: '', startedAt: new Date().toISOString(),
        endedAt: null, status: 'recording', documentId: null, hasAudio: false, autoCheck: autoCheck ?? true, lines: 0, trueCount: 0, falseCount: 0, partlyCount: 0, unverifiedCount: 0 };
      live.sessions.unshift(session);
      setState({ sessionId: session.id, investigationId, phase: 'recording', startedAt: session.startedAt, behind: 0, voices: live.consent, checking: true });
      act(session.id);
      return session.id;
    },
    live_stop: stop,
    debate_start: ({ title, participants, autoCheck }) => {
      if (live.state.phase !== 'idle') throw new Error('A live session is already running');
      const debaters = (participants as { targetId: string | null; name: string }[]).map(p => p.targetId ?? people.follow(p.name)).map(id => personOf(people.known().find(k => k.targetId === id)!));
      const session: LiveSessionSummary = { id: `live-${Date.now()}`, investigationId: null, targetId: null, targetName: debaters.map(p => p.name).join(' × '), title, notes: '',
        startedAt: new Date().toISOString(), endedAt: null, status: 'recording', documentId: null, hasAudio: false, autoCheck: autoCheck ?? true, lines: 0, trueCount: 0,
        falseCount: 0, partlyCount: 0, unverifiedCount: 0 };
      live.sessions.unshift(session);
      live.debaters.set(session.id, debaters);
      setState({ sessionId: session.id, investigationId: null, phase: 'recording', startedAt: session.startedAt, behind: 0, voices: true, checking: true });
      act(session.id, debaters);
      return session.id;
    },
    debate_list: () => live.sessions.filter(s => !s.investigationId).map(counted),
    debate_people: () => people.known(),
    debate_add_participant: ({ sessionId, name }) => {
      const id = people.follow(name);
      const debaters = live.debaters.get(sessionId) ?? [];
      if (!debaters.some(p => p.id === id)) debaters.push(personOf(people.known().find(k => k.targetId === id)!));
      live.debaters.set(sessionId, debaters);
      return id;
    },
    live_list_sessions: ({ investigationId }) => live.sessions.filter(s => s.investigationId === investigationId).map(counted),
    live_get_session: ({ id }) => {
      const session = live.sessions.find(s => s.id === id);
      if (!session) throw new Error('Session not found');
      return { session: counted(session), people: peopleOf(session), lines: live.lines.filter(l => l.sessionId === id),
        voices: live.voices.filter(v => v.sessionId === id).map(v => voiceView(v).view), checks: live.checks.filter(c => c.sessionId === id) };
    },
    live_update_session: ({ id, title, notes, autoCheck }) => {
      const session = live.sessions.find(s => s.id === id);
      if (session) Object.assign(session, title ? { title } : {}, notes != null ? { notes } : {}, autoCheck != null ? { autoCheck } : {});
    },
    live_delete_session: ({ id }) => {
      live.sessions = live.sessions.filter(s => s.id !== id);
    },
    live_resolve_voice: ({ sessionId, voiceId, targetId, label, role }) => {
      const voice = live.voices.find(v => v.sessionId === sessionId && v.voiceId === voiceId);
      if (voice) Object.assign(voice, { targetId, label, role, resolved: true, recognized: false, suggestions: [] });
    },
    live_set_line_voice: ({ lineId, voiceId }) => {
      const line = live.lines.find(l => l.id === lineId);
      if (line) line.voiceId = voiceId;
    },
    live_set_check_status: ({ id, status }) => {
      const check = live.checks.find(c => c.id === id);
      if (check) check.status = status;
    },
    live_check_now: ({ id }) => settle(id),
    live_detect_person: () => {},
    live_clip: () => {
      throw new Error('Demonstração: as sessões não guardam áudio');
    },
    live_voice_sample: () => {
      throw new Error('Demonstração: as sessões não guardam áudio');
    },
  };
  return handlers;
}
