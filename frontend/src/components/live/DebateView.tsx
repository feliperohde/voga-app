'use client';

import { useMemo, useState } from 'react';
import { Mic2, Search, UserPlus } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { FactCheckCard, startCheck, VERDICT_STYLE } from '@/components/live/FactCheckCard';
import { useI18n } from '@/i18n';
import { Answer, debateOf, Item, Panel } from '@/lib/debate';
import { formatClock } from '@/services/investigationService';
import { LiveCheck, LiveLine, LiveVoice } from '@/services/liveService';

interface DebateViewProps {
  lines: LiveLine[];
  voices: LiveVoice[];
  checks: LiveCheck[];
  /** The people followed: id → name. */
  names: Map<string, string>;
  /** A debate's people, in order: their panels show before they speak. */
  people?: string[];
  hasAudio: boolean;
  /** Voices are not told apart: there is nobody to put in panels. */
  noVoices: boolean;
  onPlay: (start: number, end: number) => void;
  onCheckChanged: (check: LiveCheck) => void;
  /** Adds a debater known only by the name given to their voice to the people followed. */
  onFollow: (panel: Panel) => Promise<void>;
}

function ItemRow({ item, hasAudio, onCheckChanged }: { item: Item; hasAudio: boolean; onCheckChanged: (check: LiveCheck) => void }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const { check } = item;
  const style = check.verdict === 'detected' || check.verdict === 'checking' ? null : VERDICT_STYLE[check.verdict];
  const denial = check.kind === 'denial';
  return (
    <div className={`rounded-md border ${style && check.status !== 'rejected' ? style.border : 'border-gray-200'}`}>
      <div className="flex items-start gap-2 p-2">
        <button onClick={() => setOpen(!open)} className="flex min-w-0 flex-1 items-start gap-2 text-left" aria-expanded={open}>
          <span className={`flex-shrink-0 rounded px-1.5 py-0.5 text-xs font-medium ${denial ? 'bg-purple-50 text-purple-800' : 'bg-blue-50 text-blue-800'}`}>
            {t(denial ? 'live.debate.denial' : 'live.debate.affirmation', { number: item.number })}
          </span>
          <span className="text-sm text-gray-800">{check.claim}</span>
        </button>
        <span className="flex-shrink-0">
          {check.verdict === 'detected' ? (
            <Button variant="outline" size="sm" className="h-7 px-2 text-xs" onClick={() => startCheck(check, onCheckChanged, t)}>
              <Search className="h-3 w-3" /> {t('live.checkNow')}
            </Button>
          ) : check.verdict === 'checking' ? (
            <ThinkingOrb variant="thinking" size={16} />
          ) : (
            <button onClick={() => setOpen(!open)} className={`rounded px-1.5 py-0.5 text-xs font-medium ${check.status === 'rejected' ? 'bg-gray-100 text-gray-500' : style!.tone}`}>
              {t(check.status === 'rejected' ? 'live.verdict.rejected' : check.status === 'confirmed' ? style!.confirmed : style!.suggested)}
            </button>
          )}
        </span>
      </div>
      {open && (
        <div className="border-t border-gray-100 p-2">
          <FactCheckCard check={check} person={null} hasAudio={hasAudio} onChanged={onCheckChanged} />
        </div>
      )}
    </div>
  );
}

function AnswerBlock({ answer, hasAudio, onPlay, onCheckChanged }: { answer: Answer; hasAudio: boolean; onPlay: DebateViewProps['onPlay']; onCheckChanged: DebateViewProps['onCheckChanged'] }) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const text = answer.turns.map(turn => turn.text).join(' ');
  const start = answer.turns[0]?.start ?? answer.items[0]?.check.start ?? 0;
  const end = answer.turns[answer.turns.length - 1]?.end ?? start;
  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-gray-500">
        {answer.question != null ? t('live.debate.answer', { number: answer.question }) : t('live.debate.opening')}
        <button onClick={() => onPlay(start, end)} disabled={!hasAudio} className="font-mono font-normal normal-case hover:text-gray-900 disabled:hover:text-gray-500">
          {formatClock(start)}
        </button>
      </div>
      {text && (
        <button onClick={() => setExpanded(!expanded)} className={`block select-text text-left text-sm text-gray-600 ${expanded ? '' : 'line-clamp-3'}`}>
          {text}
        </button>
      )}
      {answer.items.map(item => (
        <ItemRow key={item.check.id} item={item} hasAudio={hasAudio} onCheckChanged={onCheckChanged} />
      ))}
    </div>
  );
}

function PersonPanel({ panel, names, ...props }: { panel: Panel } & Pick<DebateViewProps, 'names' | 'hasAudio' | 'onPlay' | 'onCheckChanged' | 'onFollow'>) {
  const { t } = useI18n();
  const [following, setFollowing] = useState(false);
  const name = (panel.targetId && names.get(panel.targetId)) || panel.label || t('live.voice.number', { number: panel.voices[0]?.number ?? 0 });
  const items = panel.answers.flatMap(a => a.items);
  const fakes = items.filter(i => i.check.verdict === 'false').length;
  return (
    <section className="space-y-3 rounded-lg border border-gray-200 bg-white p-4 shadow-sm">
      <header className="flex flex-wrap items-center gap-2">
        <h3 className="text-base font-semibold text-gray-900">{name}</h3>
        {panel.voices.some(v => v.recognized) && <span className="text-xs text-gray-500">{t('live.voice.byVoice')}</span>}
        {!panel.targetId && panel.label && (
          <Button
            variant="outline"
            size="sm"
            className="h-7 px-2 text-xs"
            disabled={following}
            onClick={() => {
              setFollowing(true);
              props.onFollow(panel).finally(() => setFollowing(false));
            }}
          >
            <UserPlus className="h-3.5 w-3.5" /> {t('live.debate.follow')}
          </Button>
        )}
        <span className="ml-auto text-xs text-gray-500">
          {t('live.debate.counts', { items: items.length, fakes })}
        </span>
      </header>
      {!panel.targetId && !panel.label && <p className="text-xs text-gray-500">{t('live.debate.whoIs')}</p>}
      {panel.answers.length === 0 ? (
        <p className="text-sm text-gray-500">{t('live.debate.silent')}</p>
      ) : (
        panel.answers.map(answer => <AnswerBlock key={answer.question ?? 'opening'} answer={answer} {...props} />)
      )}
    </section>
  );
}

/** The session as a debate: a panel per person, with their answers to each numbered question and
 * the claims they made, checked when asked; the interviewer's questions below. */
export function DebateView({ lines, voices, checks, names, people, hasAudio, noVoices, onPlay, onCheckChanged, onFollow }: DebateViewProps) {
  const { t } = useI18n();
  const debate = useMemo(() => debateOf(lines, voices, checks, people), [lines, voices, checks, people]);
  if (noVoices) return <p className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">{t('live.debate.noVoices')}</p>;
  const hasInterviewer = voices.some(v => v.role === 'interviewer');
  return (
    <div className="space-y-4">
      {debate.panels.length === 0 ? (
        <p className="text-sm text-gray-500">{t('live.debate.empty')}</p>
      ) : (
        <div className="grid gap-4 md:grid-cols-2">
          {debate.panels.map(panel => (
            <PersonPanel key={panel.key} panel={panel} names={names} hasAudio={hasAudio} onPlay={onPlay} onCheckChanged={onCheckChanged} onFollow={onFollow} />
          ))}
        </div>
      )}
      <section className="space-y-2 rounded-lg border border-gray-200 bg-white p-4 shadow-sm">
        <h3 className="flex items-center gap-2 text-base font-semibold text-gray-900">
          <Mic2 className="h-4 w-4" /> {t('live.debate.interviewer')}
        </h3>
        {!hasInterviewer ? (
          <p className="text-sm text-gray-500">{t('live.debate.noInterviewer')}</p>
        ) : debate.questions.length === 0 ? (
          <p className="text-sm text-gray-500">{t('live.debate.noQuestions')}</p>
        ) : (
          <ol className="space-y-1.5">
            {debate.questions.map(({ number, turn }) => (
              <li key={turn.start} className={`flex gap-2 text-sm ${number == null ? 'text-gray-400' : 'text-gray-800'}`}>
                <span className="w-6 flex-shrink-0 text-right font-semibold">{number != null ? `${number}.` : ''}</span>
                <span className="min-w-0 flex-1 select-text">{turn.text}</span>
                <button onClick={() => onPlay(turn.start, turn.end)} disabled={!hasAudio} className="flex-shrink-0 font-mono text-xs text-gray-400 hover:text-gray-900">
                  {formatClock(turn.start)}
                </button>
              </li>
            ))}
          </ol>
        )}
      </section>
    </div>
  );
}
