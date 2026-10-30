import type { ClaimKind, LiveCheck, LiveLine, LiveVoice } from '@/services/liveService';

/** Consecutive lines of the same voice. */
export interface Turn {
  voiceId: string;
  start: number;
  end: number;
  text: string;
}

/** A turn of the interviewer: numbered when it asks something ("?"). */
export interface Question {
  number: number | null;
  turn: Turn;
}

/** A claim of a person, numbered among their affirmations or their denials. */
export interface Item {
  check: LiveCheck;
  number: number;
}

/** What a person said after question N (null: before the first question). */
export interface Answer {
  question: number | null;
  turns: Turn[];
  items: Item[];
}

/** A person debating: a person followed, a voice named by the user, or a voice not answered yet. */
export interface Panel {
  key: string;
  targetId: string | null;
  label: string | null;
  voices: LiveVoice[];
  answers: Answer[];
}

export interface Debate {
  questions: Question[];
  panels: Panel[];
}

/** Who a voice is in the debate: the same person may have more than one voice. */
const keyOf = (voice: LiveVoice) => (voice.targetId ? `t:${voice.targetId}` : voice.label ? `l:${voice.label.toLowerCase()}` : `v:${voice.voiceId}`);

/** The debate a session's lines, voices and checks make: the interviewer's questions numbered, and
 * for each person their answers to each question with the claims they made in them. `people`: a
 * debate's people (target ids), whose panels come first, in order, before they speak. */
export function debateOf(lines: LiveLine[], voices: LiveVoice[], checks: LiveCheck[], people: string[] = []): Debate {
  const byId = new Map(voices.map(v => [v.voiceId, v]));
  const turns: Turn[] = [];
  for (const line of [...lines].sort((a, b) => a.start - b.start)) {
    // A line of a voice never announced (noise, a cough) has nobody to go to
    if (!line.voiceId || !byId.has(line.voiceId)) continue;
    const last = turns[turns.length - 1];
    if (last && last.voiceId === line.voiceId) {
      last.end = line.end;
      last.text = `${last.text} ${line.text}`;
    } else {
      turns.push({ voiceId: line.voiceId, start: line.start, end: line.end, text: line.text });
    }
  }

  const questions: Question[] = [];
  let asked = 0;
  for (const turn of turns.filter(t => byId.get(t.voiceId)!.role === 'interviewer')) {
    questions.push({ number: turn.text.includes('?') ? ++asked : null, turn });
  }
  const questionAt = (time: number) => questions.reduce<number | null>((current, q) => (q.number != null && q.turn.start <= time ? q.number : current), null);

  const panels = new Map<string, Panel>();
  const panelOf = (key: string, voice: LiveVoice | null) => {
    let panel = panels.get(key);
    if (!panel) {
      panel = { key, targetId: voice?.targetId ?? null, label: voice?.label ?? null, voices: [], answers: [] };
      panels.set(key, panel);
    }
    if (voice && !panel.voices.includes(voice)) panel.voices.push(voice);
    return panel;
  };
  const answerOf = (panel: Panel, question: number | null) => {
    let answer = panel.answers.find(a => a.question === question);
    if (!answer) {
      answer = { question, turns: [], items: [] };
      panel.answers.push(answer);
    }
    return answer;
  };

  for (const targetId of people) panelOf(`t:${targetId}`, null).targetId = targetId;
  for (const turn of turns) {
    const voice = byId.get(turn.voiceId)!;
    if (voice.role === 'interviewer') continue;
    answerOf(panelOf(keyOf(voice), voice), questionAt(turn.start)).turns.push(turn);
  }

  const counters = new Map<string, Record<ClaimKind, number>>();
  for (const check of [...checks].sort((a, b) => a.start - b.start)) {
    if (!check.targetId) continue;
    const key = `t:${check.targetId}`;
    const panel = panelOf(key, null);
    panel.targetId ??= check.targetId;
    const count = counters.get(key) ?? { affirmation: 0, denial: 0 };
    count[check.kind] += 1;
    counters.set(key, count);
    answerOf(panel, questionAt(check.start)).items.push({ check, number: count[check.kind] });
  }

  for (const panel of panels.values()) {
    panel.answers.sort((a, b) => (a.question ?? 0) - (b.question ?? 0));
  }
  return { questions, panels: [...panels.values()] };
}
