'use client';

import { useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { FileText, Play, Square, Trash2, Volume2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Textarea } from '@/components/ui/textarea';
import { Switch } from '@/components/ui/switch';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { useConfirm } from '@/components/ui/confirm-dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { FactCheckCard } from '@/components/live/FactCheckCard';
import { DebateView } from '@/components/live/DebateView';
import type { Panel } from '@/lib/debate';
import { useLiveState } from '@/hooks/useLiveState';
import { formatDate, useI18n } from '@/i18n';
import { formatClock, investigationService } from '@/services/investigationService';
import { LiveCheck, LiveLine, LiveSessionSummary, liveService, LiveVoice, SessionPerson, Verdict, VoiceAnswer } from '@/services/liveService';

type Filter = 'all' | Exclude<Verdict, 'checking'>;
const FILTERS: Filter[] = ['all', 'detected', 'false', 'partly', 'true', 'unverified'];
const VIEW_KEY = 'voga.liveView';
type View = 'transcript' | 'debate';

function savedView(): View {
  try {
    return localStorage.getItem(VIEW_KEY) === 'debate' ? 'debate' : 'transcript';
  } catch {
    return 'transcript';
  }
}

function upsert<T>(list: T[], item: T, same: (a: T, b: T) => boolean): T[] {
  const at = list.findIndex(other => same(other, item));
  return at < 0 ? [...list, item] : list.map((other, i) => (i === at ? item : other));
}

function Level({ level }: { level: number }) {
  return (
    <div className="flex h-4 items-end gap-0.5" aria-hidden>
      {[0.15, 0.35, 0.55, 0.75, 0.9].map(step => (
        <span key={step} className={`w-1 rounded-sm ${level >= step ? 'bg-green-500' : 'bg-gray-200'}`} style={{ height: `${30 + step * 70}%` }} />
      ))}
    </div>
  );
}

function Elapsed({ since }: { since: string }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  return <span className="font-mono text-sm tabular-nums text-gray-700">{formatClock((now - new Date(since).getTime()) / 1000)}</span>;
}

/** "Voz 2: quem é?" — the people followed, the closest voiceprint first; a name; the interviewer; or "não sei". */
function VoicePrompt({ sessionId, voice, onAnswer }: { sessionId: string; voice: LiveVoice; onAnswer: (answer: VoiceAnswer) => void }) {
  const { t } = useI18n();
  const [name, setName] = useState('');
  return (
    <div className="space-y-2 rounded-lg border border-blue-200 bg-blue-50/60 p-3">
      <div className="flex items-center gap-2 text-sm font-medium text-gray-900">
        {t('live.voice.who', { number: voice.number })}
        <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={() => liveService.playVoice(sessionId, voice.voiceId).catch(error => toast.error(t('live.playFailed'), { description: String(error) }))}>
          <Volume2 className="mr-1 h-3.5 w-3.5" /> {t('live.voice.listen')}
        </Button>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {voice.suggestions.map(s => (
          <Button key={s.targetId} variant="outline" size="sm" className="h-7 text-xs" disabled={s.taken} onClick={() => onAnswer({ targetId: s.targetId })}>
            {s.name}
            {s.similarity != null && <span className="text-gray-500">{Math.round(s.similarity * 100)}%</span>}
          </Button>
        ))}
        <form
          className="flex gap-1"
          onSubmit={event => {
            event.preventDefault();
            if (name.trim()) onAnswer({ label: name.trim() });
          }}
        >
          <Input value={name} onChange={e => setName(e.target.value)} placeholder={t('live.voice.other')} className="h-7 w-40 text-xs" />
        </form>
        <Button variant="outline" size="sm" className="h-7 text-xs" onClick={() => onAnswer({ role: 'interviewer' })}>
          {t('live.debate.interviewer')}
        </Button>
        <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={() => onAnswer({})}>
          {t('live.voice.unknown')}
        </Button>
      </div>
    </div>
  );
}

/** A live session, going on or over: who said what, the checks of what the people followed said, and the notes. */
export function LiveSession({ id }: { id: string }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const confirm = useConfirm();
  const live = useLiveState();
  const [session, setSession] = useState<LiveSessionSummary | null>(null);
  const [lines, setLines] = useState<LiveLine[]>([]);
  const [voices, setVoices] = useState<LiveVoice[]>([]);
  const [checks, setChecks] = useState<LiveCheck[]>([]);
  const [people, setPeople] = useState<SessionPerson[]>([]);
  const [notes, setNotes] = useState<string | null>(null);
  const [level, setLevel] = useState(0);
  const [filter, setFilter] = useState<Filter>('all');
  const [view, setView] = useState<View>('transcript');
  useEffect(() => setView(savedView()), []);
  const chooseView = (next: View) => {
    setView(next);
    try {
      localStorage.setItem(VIEW_KEY, next);
    } catch {}
  };
  const [error, setError] = useState<string | null>(null);
  const transcript = useRef<HTMLDivElement | null>(null);
  const following = useRef(true);

  const mine = live?.sessionId === id;
  const recording = mine && live?.phase === 'recording';
  const finishing = mine && live?.phase === 'finishing';

  const load = () =>
    liveService
      .session(id)
      .then(detail => {
        setSession(detail.session);
        setLines(detail.lines);
        setVoices(detail.voices);
        setChecks(detail.checks);
        setNotes(current => current ?? detail.session.notes);
        setPeople(detail.people);
      })
      .catch(e => setError(String(e)));

  useEffect(() => {
    load();
    const listeners = [
      liveService.onLine(line => line.sessionId === id && setLines(list => upsert(list, line, (a, b) => a.id === b.id))),
      liveService.onVoice((sessionId, voice) => sessionId === id && setVoices(list => upsert(list, voice, (a, b) => a.voiceId === b.voiceId))),
      liveService.onCheck(check => check.sessionId === id && setChecks(list => upsert(list, check, (a, b) => a.id === b.id))),
      liveService.onLevel(setLevel),
    ];
    return () => listeners.forEach(unlisten => unlisten.then(fn => fn()));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  // The session ended: its document and status are read again
  const wasMine = useRef(false);
  useEffect(() => {
    if (mine) wasMine.current = true;
    else if (wasMine.current && live?.phase === 'idle') {
      wasMine.current = false;
      load();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mine, live?.phase]);

  // The notes are kept a moment after typing stops
  const savedNotes = useRef<string | null>(null);
  useEffect(() => {
    if (notes === null) return;
    if (savedNotes.current === null) {
      savedNotes.current = notes;
      return;
    }
    if (notes === savedNotes.current) return;
    const timer = setTimeout(() => {
      liveService
        .update(id, { notes })
        .then(() => (savedNotes.current = notes))
        .catch(e => toast.error(t('common.saveFailed'), { description: String(e) }));
    }, 800);
    return () => clearTimeout(timer);
  }, [notes, id, t]);

  // The transcript follows the speech unless the reader scrolled up
  useEffect(() => {
    const box = transcript.current;
    if (box && following.current) box.scrollTop = box.scrollHeight;
  }, [lines.length]);

  // A debate: outside every investigation, its people each checked in their own
  const debate = !!session && !session.investigationId;
  useEffect(() => {
    if (debate) setView('debate');
  }, [debate]);
  const back = debate ? '/debate' : '/live';
  const nameOf = useMemo(() => new Map(people.map(p => [p.id, p.name])), [people]);
  const debaters = useMemo(() => (debate ? people.map(p => p.id) : undefined), [debate, people]);
  const voiceName = (voice: LiveVoice | undefined) =>
    !voice
      ? '?'
      : voice.role === 'interviewer'
        ? t('live.debate.interviewer')
        : voice.targetId
          ? (nameOf.get(voice.targetId) ?? '?')
          : (voice.label ?? t('live.voice.number', { number: voice.number }));
  const voiceOf = (line: LiveLine) => voices.find(v => v.voiceId === line.voiceId);
  // Without voices told apart, every line is the candidate's
  const voicesOn = voices.length > 0 || (mine && !!live?.voices);
  const checkAt = (line: LiveLine) => checks.find(c => c.start >= line.start - 0.01 && c.start < line.end);
  const counts = Object.fromEntries(FILTERS.map(f => [f, f === 'all' ? checks.length : checks.filter(c => c.verdict === f).length])) as Record<Filter, number>;
  const shown = checks.filter(c => filter === 'all' || c.verdict === filter).sort((a, b) => b.start - a.start);
  const pending = checks.filter(c => c.verdict === 'checking').length;
  const many = new Set(checks.map(c => c.targetId)).size > 1;

  const answer = (voice: LiveVoice, given: VoiceAnswer) =>
    liveService
      .resolveVoice(id, voice.voiceId, given)
      .then(() =>
        setVoices(list =>
          list.map(v =>
            v.voiceId === voice.voiceId
              ? { ...v, targetId: given.targetId ?? null, label: given.label ?? null, role: given.role ?? null, resolved: true, recognized: false, suggestions: [] }
              : v,
          ),
        ),
      )
      .catch(e => toast.error(t('common.saveFailed'), { description: String(e) }));
  // A debater known only by a name: followed from now on (in a debate, in an investigation of
  // their own when nobody follows them), their voices given to them
  const follow = async (panel: Panel) => {
    if (!session || !panel.label) return;
    try {
      const targetId = session.investigationId
        ? await investigationService.saveTarget({ id: null, investigationId: session.investigationId, kind: 'person', name: panel.label, aliases: [] })
        : await liveService.addParticipant(id, panel.label);
      for (const voice of panel.voices) await liveService.resolveVoice(id, voice.voiceId, { targetId });
      setVoices(list => list.map(v => (panel.voices.some(p => p.voiceId === v.voiceId) ? { ...v, targetId, label: null } : v)));
      setPeople((await liveService.session(id)).people);
      if (!mine) await liveService.detectPerson(id, targetId);
      toast.success(t(debate ? 'debate.joined' : 'live.debate.followed', { name: panel.label }));
    } catch (e) {
      toast.error(t('common.saveFailed'), { description: String(e) });
    }
  };
  const setAutoCheck = (autoCheck: boolean) =>
    liveService
      .update(id, { autoCheck })
      .then(() => setSession(current => (current ? { ...current, autoCheck } : current)))
      .catch(e => toast.error(t('common.saveFailed'), { description: String(e) }));
  const moveLine = (line: LiveLine, voiceId: string) =>
    liveService
      .setLineVoice(line.id, voiceId)
      .then(() => setLines(list => list.map(l => (l.id === line.id ? { ...l, voiceId } : l))))
      .catch(e => toast.error(t('common.saveFailed'), { description: String(e) }));
  const stop = () => liveService.stop().catch(e => toast.error(t('live.stopFailed'), { description: String(e) }));
  const remove = async () => {
    if (await confirm({ title: t('live.deleteTitle'), description: t('live.deleteHelp'), destructive: true, confirmLabel: t('common.delete') })) {
      liveService
        .remove(id)
        .then(() => router.push(back))
        .catch(e => toast.error(String(e)));
    }
  };
  const play = (start: number, end: number) =>
    session?.hasAudio && liveService.playClip(id, start, end).catch(e => toast.error(t('live.playFailed'), { description: String(e) }));

  if (error) return <p className="p-8 text-sm text-red-600">{error}</p>;
  if (!session) {
    return (
      <div className="flex min-h-[60vh] items-center justify-center">
        <ThinkingOrb variant="working" size={28} />
      </div>
    );
  }

  const notices = [
    recording && !live?.checking && t('live.notice.noModel'),
    recording && !live?.voices && !debate && t('live.notice.noVoices', { name: session.targetName ?? '' }),
    recording && (live?.behind ?? 0) > 15 && t('live.notice.behind', { seconds: Math.round(live?.behind ?? 0) }),
    session.status === 'interrupted' && t('live.notice.interrupted'),
  ].filter(Boolean) as string[];

  return (
    <div>
      <PageHeader
        title={session.title}
        onBack={() => router.push(back)}
        actions={
          recording ? (
            <>
              <Level level={level} />
              <Elapsed since={session.startedAt} />
              <Button variant="red" size="sm" onClick={stop}>
                <Square className="mr-1 h-3.5 w-3.5 fill-current" /> {t('live.stop')}
              </Button>
            </>
          ) : finishing ? (
            <span className="flex items-center gap-2 text-sm text-gray-600">
              <ThinkingOrb variant="background" size={16} /> {t('live.finishing')}
            </span>
          ) : (
            <>
              {session.documentId && (
                <Button variant="outline" size="sm" onClick={() => router.push(`/document?id=${encodeURIComponent(session.documentId!)}`)}>
                  <FileText className="mr-1 h-4 w-4" /> {t('live.openDocument')}
                </Button>
              )}
              <Button variant="ghost" size="icon" title={t('common.delete')} onClick={remove}>
                <Trash2 className="h-4 w-4" />
              </Button>
            </>
          )
        }
      />
      <div className="mx-auto max-w-6xl space-y-4 px-4 py-6 sm:px-8">
        <p className="text-sm text-gray-600">
          {[session.targetName, formatDate(locale, session.startedAt, { dateStyle: 'medium', timeStyle: 'short' })].filter(Boolean).join(' · ')}
        </p>
        {notices.map(notice => (
          <p key={notice} className="rounded-md border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">
            {notice}
          </p>
        ))}

        <div className="flex flex-wrap items-center gap-4">
          <Tabs value={view} onValueChange={next => chooseView(next as View)}>
            <TabsList>
              <TabsTrigger value="transcript">{t('live.view.transcript')}</TabsTrigger>
              <TabsTrigger value="debate">{t('live.view.debate')}</TabsTrigger>
            </TabsList>
          </Tabs>
          <label className="ml-auto flex items-center gap-2 text-sm text-gray-700" title={t('live.autoCheckHelp')}>
            <Switch checked={session.autoCheck} onCheckedChange={setAutoCheck} aria-label={t('live.autoCheck')} />
            {t('live.autoCheck')}
          </label>
        </div>
        {voices
          .filter(v => !v.resolved)
          .map(voice => (
            <VoicePrompt key={voice.voiceId} sessionId={id} voice={voice} onAnswer={given => answer(voice, given)} />
          ))}

        {view === 'debate' ? (
          <DebateView
            lines={lines}
            voices={voices}
            checks={checks}
            names={nameOf}
            people={debaters}
            hasAudio={session.hasAudio}
            noVoices={!voicesOn}
            onPlay={play}
            onCheckChanged={changed => setChecks(list => upsert(list, changed, (a, b) => a.id === b.id))}
            onFollow={follow}
          />
        ) : (
        <div className="grid gap-6 lg:grid-cols-5">
          <section className="space-y-3 lg:col-span-3">
            <h2 className="text-sm font-semibold uppercase tracking-wide text-gray-500">{t('live.transcript')}</h2>
            <div
              ref={transcript}
              onScroll={event => {
                const box = event.currentTarget;
                following.current = box.scrollHeight - box.scrollTop - box.clientHeight < 80;
              }}
              className="max-h-[60vh] space-y-0.5 overflow-y-auto rounded-lg border border-gray-200 bg-white p-2"
            >
              {lines.length === 0 && (
                <p className="flex items-center gap-2 p-4 text-sm text-gray-500">
                  {recording && <ThinkingOrb variant="background" size={16} />}
                  {t(recording ? 'live.listening' : 'live.noLines')}
                </p>
              )}
              {lines.map(line => {
                const voice = voiceOf(line);
                const followed = !voicesOn || (!!voice?.targetId && nameOf.has(voice.targetId));
                const check = checkAt(line);
                return (
                  <div key={line.id} className="group flex gap-2 rounded px-2 py-1 text-sm hover:bg-gray-50">
                    <button onClick={() => play(line.start, line.end)} className="flex w-12 flex-shrink-0 items-start gap-0.5 pt-0.5 font-mono text-xs text-gray-400 hover:text-gray-900" title={t('live.listen')}>
                      {formatClock(line.start)}
                      <Play className="mt-0.5 hidden h-2.5 w-2.5 group-hover:block" />
                    </button>
                    {voicesOn && (
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <button className={`h-fit flex-shrink-0 rounded px-1.5 py-0.5 text-xs font-medium ${followed ? 'bg-gray-900 text-white' : 'bg-gray-100 text-gray-600'}`}>
                            {voiceName(voice)}
                            {voice?.recognized && <span className="ml-1 opacity-70">{t('live.voice.byVoice')}</span>}
                          </button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="start" className="w-56">
                          {voice && (
                            <>
                              <DropdownMenuLabel>{t('live.voice.is', { voice: t('live.voice.number', { number: voice.number }) })}</DropdownMenuLabel>
                              {people.map(person => (
                                <DropdownMenuItem key={person.id} onSelect={() => answer(voice, { targetId: person.id })}>
                                  {person.name}
                                </DropdownMenuItem>
                              ))}
                              <DropdownMenuItem onSelect={() => answer(voice, { role: 'interviewer' })}>{t('live.debate.interviewer')}</DropdownMenuItem>
                              <DropdownMenuItem onSelect={() => answer(voice, {})}>{t('live.voice.unknown')}</DropdownMenuItem>
                            </>
                          )}
                          {voices.filter(v => v.voiceId !== line.voiceId).length > 0 && (
                            <>
                              <DropdownMenuSeparator />
                              <DropdownMenuLabel>{t('live.voice.lineIs')}</DropdownMenuLabel>
                              {voices
                                .filter(v => v.voiceId !== line.voiceId)
                                .map(v => (
                                  <DropdownMenuItem key={v.voiceId} onSelect={() => moveLine(line, v.voiceId)}>
                                    {voiceName(v)}
                                  </DropdownMenuItem>
                                ))}
                            </>
                          )}
                        </DropdownMenuContent>
                      </DropdownMenu>
                    )}
                    <span className={`min-w-0 flex-1 select-text ${followed ? 'text-gray-900' : 'text-gray-400'}`}>
                      {line.text}
                      {check && (
                        <a href={`#check-${check.id}`} className="ml-1.5 inline-block h-2 w-2 rounded-full bg-amber-400 align-middle" title={check.claim} />
                      )}
                    </span>
                  </div>
                );
              })}
            </div>
          </section>

          <section className="space-y-3 lg:col-span-2">
            <div>
              <h2 className="text-sm font-semibold uppercase tracking-wide text-gray-500">{t('live.checks')}</h2>
              <p className="text-xs text-gray-500">{t('live.checksHelp')}</p>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {FILTERS.map(f => (
                <Button key={f} variant={filter === f ? 'default' : 'outline'} size="sm" className="h-7 text-xs" onClick={() => setFilter(f)}>
                  {t(`live.filter.${f}`)} <span className="opacity-70">{counts[f]}</span>
                </Button>
              ))}
            </div>
            {pending > 0 && (
              <p className="flex items-center gap-2 text-xs text-gray-600">
                <ThinkingOrb variant="thinking" size={14} /> {t('live.checking', { count: pending })}
              </p>
            )}
            {shown.length === 0 ? (
              <p className="py-6 text-center text-sm text-gray-500">{t(checks.length ? 'live.noChecksInView' : 'live.noChecks')}</p>
            ) : (
              shown.map(check => (
                <FactCheckCard
                  key={check.id}
                  check={check}
                  person={many && check.targetId ? (nameOf.get(check.targetId) ?? null) : null}
                  hasAudio={session.hasAudio}
                  onChanged={changed => setChecks(list => upsert(list, changed, (a, b) => a.id === b.id))}
                />
              ))
            )}
          </section>
        </div>
        )}

        <div className="space-y-1.5">
          <h2 className="text-sm font-semibold uppercase tracking-wide text-gray-500">{t('live.notes')}</h2>
          <Textarea value={notes ?? ''} onChange={e => setNotes(e.target.value)} placeholder={t('live.notesPlaceholder')} rows={4} />
        </div>
      </div>
    </div>
  );
}
