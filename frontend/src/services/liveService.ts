import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import type { Actor } from './claimsService';
import type { DocumentKind } from './investigationService';

/** idle | recording | finishing (the last lines are transcribed and the document saved). */
export type LivePhase = 'idle' | 'recording' | 'finishing';

export interface LiveState {
  sessionId: string | null;
  investigationId: string | null;
  phase: LivePhase;
  startedAt: string | null;
  /** Seconds the transcript is behind what is said. */
  behind: number;
  /** Voices are told apart. */
  voices: boolean;
  /** Claims are checked (an AI model is set up). */
  checking: boolean;
}

export interface Microphone {
  name: string;
  isDefault: boolean;
}

export interface LiveStatus {
  state: LiveState;
  /** The Whisper model a session would use here; null: none downloaded. */
  transcriptionModel: string | null;
  /** 'builtin' (Whisper here) or the provider that transcribes, with its model. */
  transcriptionProvider: string;
  transcriptionRemoteModel: string;
  voiceModel: boolean;
  voiceConsent: boolean;
  /** This computer can hear its own sound (macOS 14.2+, Windows). */
  systemAudio: boolean;
  microphones: Microphone[];
  chatModel: boolean;
}

export interface LiveSessionSummary {
  id: string;
  /** null: a debate, outside every investigation. */
  investigationId: string | null;
  targetId: string | null;
  /** The candidate, or a debate's people ("A × B"). */
  targetName: string | null;
  title: string;
  notes: string;
  startedAt: string;
  endedAt: string | null;
  status: 'recording' | 'done' | 'interrupted';
  documentId: string | null;
  hasAudio: boolean;
  /** Claims are checked as found; off, they wait for "Checar". */
  autoCheck: boolean;
  lines: number;
  trueCount: number;
  falseCount: number;
  partlyCount: number;
  unverifiedCount: number;
}

/** A line of the transcript, in seconds from the session start. */
export interface LiveLine {
  id: number;
  sessionId: string;
  start: number;
  end: number;
  text: string;
  voiceId: string | null;
}

export interface VoiceSuggestion {
  targetId: string;
  name: string;
  similarity: number | null;
  /** Already given to another voice of the session. */
  taken: boolean;
}

export interface LiveVoice {
  voiceId: string;
  number: number;
  targetId: string | null;
  label: string | null;
  /** 'interviewer': asks the questions (never checked). */
  role: 'interviewer' | null;
  /** Answered ("não sei" included), by the user or by a voiceprint. */
  resolved: boolean;
  /** Named by a voiceprint, not by the user. */
  recognized: boolean;
  seconds: number;
  heard: [number, number][];
  suggestions: VoiceSuggestion[];
}

/** detected: found, waiting for "Checar"; checking: the records are being read. */
export type Verdict = 'detected' | 'checking' | 'true' | 'false' | 'partly' | 'unverified';
/** A claim affirms something, or denies a fact or an accusation. */
export type ClaimKind = 'affirmation' | 'denial';
export type CheckStatus = 'suggested' | 'confirmed' | 'rejected';

/** A record a verdict rests on. */
export interface Evidence {
  documentId: string;
  chunkId: string;
  title: string;
  kind: DocumentKind;
  date: string;
  url: string;
  site: string | null;
  excerpt: string;
  startTime: number | null;
  actor: Actor | null;
}

export interface LiveCheck {
  id: string;
  sessionId: string;
  targetId: string | null;
  start: number;
  end: number;
  quote: string;
  claim: string;
  topic: string;
  kind: ClaimKind;
  verdict: Verdict;
  confidence: number | null;
  rationale: string | null;
  evidence: Evidence[];
  webSearched: boolean;
  status: CheckStatus;
  error: string | null;
}

/** A person a session follows (a debate's people in order), and where their records are. */
export interface SessionPerson {
  id: string;
  name: string;
  investigationId: string;
}

export interface LiveSessionDetail {
  session: LiveSessionSummary;
  people: SessionPerson[];
  lines: LiveLine[];
  voices: LiveVoice[];
  checks: LiveCheck[];
}

export interface VoiceAnswer {
  targetId?: string | null;
  label?: string | null;
  role?: 'interviewer' | null;
}

export interface StartOptions {
  investigationId: string;
  targetId: string;
  title: string;
  /** A microphone's name ("" for the default one), or null for none. */
  microphone: string | null;
  systemAudio: boolean;
  /** Check each claim as found (else only detect it). */
  autoCheck: boolean;
}

/** A person followed in some investigation, to pick for a debate. */
export interface KnownPerson {
  targetId: string;
  name: string;
  aliases: string[];
  investigationId: string;
  investigationName: string;
}

/** A person of a new debate: one followed (targetId), or a name nobody follows yet. */
export interface DebateParticipant {
  targetId: string | null;
  name: string;
}

export interface DebateOptions {
  title: string;
  participants: DebateParticipant[];
  microphone: string | null;
  systemAudio: boolean;
  autoCheck: boolean;
}

let player: HTMLAudioElement | null = null;

/** Plays WAV bytes from the core, stopping what was playing. */
async function play(wav: Promise<ArrayBuffer>) {
  const bytes = await wav;
  if (player) {
    player.pause();
    URL.revokeObjectURL(player.src);
  }
  player = new Audio(URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' })));
  await player.play();
}

export const liveService = {
  status: () => invoke<LiveStatus>('live_status'),
  saveSettings: (settings: { voiceConsent: boolean; transcriptionModel: string | null; transcriptionProvider?: string; transcriptionRemoteModel?: string }) =>
    invoke<void>('live_save_settings', { settings }),
  /** Resolves with the session's id once the sound is being heard. */
  start: (options: StartOptions) => invoke<string>('live_start', { ...options }),
  stop: () => invoke<void>('live_stop'),
  /** A person nobody follows gets an investigation of their own before the debate starts. */
  startDebate: (options: DebateOptions) => invoke<string>('debate_start', { ...options }),
  debates: () => invoke<LiveSessionSummary[]>('debate_list'),
  debatePeople: () => invoke<KnownPerson[]>('debate_people'),
  /** Someone named in a debate joins its people (followed in an investigation of their own when nobody follows them). */
  addParticipant: (sessionId: string, name: string) => invoke<string>('debate_add_participant', { sessionId, name }),
  sessions: (investigationId: string) => invoke<LiveSessionSummary[]>('live_list_sessions', { investigationId }),
  session: (id: string) => invoke<LiveSessionDetail>('live_get_session', { id }),
  update: (id: string, changes: { title?: string; notes?: string; autoCheck?: boolean }) =>
    invoke<void>('live_update_session', { id, title: changes.title ?? null, notes: changes.notes ?? null, autoCheck: changes.autoCheck ?? null }),
  remove: (id: string) => invoke<void>('live_delete_session', { id }),
  /** Who a voice is: a person followed, a name, the interviewer, or neither ("não sei"). */
  resolveVoice: (sessionId: string, voiceId: string, answer: VoiceAnswer) =>
    invoke<void>('live_resolve_voice', { sessionId, voiceId, targetId: answer.targetId ?? null, label: answer.label ?? null, role: answer.role ?? null }),
  /** Checks one claim now (detected, or again); the verdict comes as a live-check event. */
  checkNow: (id: string) => invoke<void>('live_check_now', { id }),
  /** After a session: finds the claims of someone just given a voice. */
  detectPerson: (sessionId: string, targetId: string) => invoke<void>('live_detect_person', { sessionId, targetId }),
  setLineVoice: (lineId: number, voiceId: string | null) => invoke<void>('live_set_line_voice', { lineId, voiceId }),
  setCheckStatus: (id: string, status: CheckStatus) => invoke<void>('live_set_check_status', { id, status }),
  playClip: (sessionId: string, start: number, end: number) => play(invoke<ArrayBuffer>('live_clip', { sessionId, start, end })),
  playVoice: (sessionId: string, voiceId: string) => play(invoke<ArrayBuffer>('live_voice_sample', { sessionId, voiceId })),
  onState: (handler: (state: LiveState) => void): Promise<UnlistenFn> => listen<LiveState>('live-state', e => handler(e.payload)),
  onLevel: (handler: (level: number) => void): Promise<UnlistenFn> => listen<{ level: number }>('live-level', e => handler(e.payload.level)),
  onLine: (handler: (line: LiveLine) => void): Promise<UnlistenFn> => listen<LiveLine>('live-transcript', e => handler(e.payload)),
  onVoice: (handler: (sessionId: string, voice: LiveVoice) => void): Promise<UnlistenFn> =>
    listen<{ sessionId: string; voice: LiveVoice }>('live-voice', e => handler(e.payload.sessionId, e.payload.voice)),
  onCheck: (handler: (check: LiveCheck) => void): Promise<UnlistenFn> => listen<LiveCheck>('live-check', e => handler(e.payload)),
};

/** A session can start: Whisper is downloaded here, or a provider transcribes with a model chosen. */
export function transcriptionReady(status: LiveStatus): boolean {
  return status.transcriptionProvider === 'builtin' ? !!status.transcriptionModel : !!status.transcriptionRemoteModel;
}

/** Where a session is shown: the Debate page for a debate, Ao vivo otherwise. */
export function sessionHref(id: string, debate: boolean): string {
  return `${debate ? '/debate' : '/live'}?id=${encodeURIComponent(id)}`;
}

/** The session a `live` document came from (its address is voga://live/<id>). */
export function sessionOfDocument(url: string): string | null {
  return url.startsWith('voga://live/') ? url.slice('voga://live/'.length) : null;
}
