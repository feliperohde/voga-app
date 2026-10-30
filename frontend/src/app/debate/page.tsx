'use client';

import { Suspense, useCallback, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { Check, Swords } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { Capture, CaptureOptions, initialCapture, microphoneOf } from '@/components/live/CaptureOptions';
import { LiveSession } from '@/components/live/LiveSession';
import { LiveSettings } from '@/components/live/LiveSettings';
import { SessionRow } from '@/components/live/SessionRow';
import { useI18n } from '@/i18n';
import { KnownPerson, LiveSessionSummary, liveService, LiveStatus, sessionHref, transcriptionReady } from '@/services/liveService';

const MAX_PEOPLE = 8;

/** A person of the debate being set up: the name typed and, when picked, who they are. */
interface Seat {
  name: string;
  targetId: string | null;
}

const EMPTY: Seat = { name: '', targetId: null };
const plain = (text: string) => text.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().trim();
const namesOf = (person: KnownPerson) => [person.name, ...person.aliases].map(plain);

/** A debate tells the voices apart, so it needs the voice model and the consent besides the transcription. */
const debateReady = (status: LiveStatus) => transcriptionReady(status) && status.voiceConsent && status.voiceModel;

function SeatField({ index, seat, known, onChange }: { index: number; seat: Seat; known: KnownPerson[]; onChange: (seat: Seat) => void }) {
  const { t } = useI18n();
  const typed = plain(seat.name);
  // Who the name is: the one picked, else someone followed under that very name (as the core links it)
  const person = seat.targetId ? known.find(k => k.targetId === seat.targetId) : typed ? known.find(k => namesOf(k).includes(typed)) : undefined;
  const others = person ? known.filter(k => k.targetId !== person.targetId && namesOf(k).includes(plain(person.name))) : [];
  const matches = !person && typed.length >= 3 ? known.filter(k => namesOf(k).some(n => n.includes(typed))).slice(0, 4) : [];
  return (
    <div className="space-y-1.5 rounded-md border border-gray-200 p-3">
      <Label htmlFor={`debate-seat-${index}`}>{t('debate.person', { number: index + 1 })}</Label>
      <Input id={`debate-seat-${index}`} value={seat.name} onChange={e => onChange({ name: e.target.value, targetId: null })} placeholder={t('debate.namePlaceholder')} />
      {person ? (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
          <span className="flex items-center gap-1 text-green-800">
            <Check className="h-3.5 w-3.5" /> {t('debate.uses', { investigation: person.investigationName })}
          </span>
          {others.length > 0 && <span className="text-gray-500">{t('debate.alsoIn')}</span>}
          {others.map(other => (
            <button key={other.targetId} onClick={() => onChange({ name: other.name, targetId: other.targetId })} className="rounded bg-gray-100 px-1.5 py-0.5 text-gray-700 hover:bg-gray-200">
              {other.investigationName}
            </button>
          ))}
        </div>
      ) : (
        typed && (
          <>
            {matches.length > 0 && (
              <div className="flex flex-wrap gap-1">
                {matches.map(match => (
                  <Button key={match.targetId} variant="outline" size="sm" className="h-6 px-2 text-xs" onClick={() => onChange({ name: match.name, targetId: match.targetId })}>
                    {match.name} <span className="text-gray-500">· {match.investigationName}</span>
                  </Button>
                ))}
              </div>
            )}
            <p className="text-xs text-gray-600">{t('debate.newInvestigation', { name: seat.name.trim() })}</p>
          </>
        )
      )}
    </div>
  );
}

function NewDebate({ status, known }: { status: LiveStatus; known: KnownPerson[] }) {
  const { t } = useI18n();
  const router = useRouter();
  const [title, setTitle] = useState('');
  const [seats, setSeats] = useState<Seat[]>([EMPTY, EMPTY]);
  const [capture, setCapture] = useState<Capture>(() => initialCapture(status));
  const [starting, setStarting] = useState(false);

  const resize = (count: number) => setSeats(list => Array.from({ length: count }, (_, i) => list[i] ?? EMPTY));
  const start = () => {
    setStarting(true);
    const participants = seats.map(seat => {
      const typed = plain(seat.name);
      const person = seat.targetId ? known.find(k => k.targetId === seat.targetId) : known.find(k => namesOf(k).includes(typed));
      return { targetId: person?.targetId ?? null, name: person?.name ?? seat.name.trim() };
    });
    liveService
      .startDebate({
        title: title.trim() || t('debate.defaultTitle', { names: participants.map(p => p.name).join(' × ') }),
        participants,
        microphone: microphoneOf(capture),
        systemAudio: capture.system,
        autoCheck: capture.autoCheck,
      })
      .then(id => router.push(sessionHref(id, true)))
      .catch(e => toast.error(t('live.startFailed'), { description: String(e) }))
      .finally(() => setStarting(false));
  };

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="debate-count">{t('debate.count')}</Label>
          <SelectField
            id="debate-count"
            value={String(seats.length)}
            onValueChange={count => resize(Number(count))}
            options={Array.from({ length: MAX_PEOPLE }, (_, i) => ({ value: String(i + 1), label: String(i + 1) }))}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="debate-title">{t('live.sessionTitle')}</Label>
          <Input id="debate-title" value={title} onChange={e => setTitle(e.target.value)} placeholder={t('live.sessionTitlePlaceholder')} />
        </div>
      </div>
      <p className="text-xs text-gray-600">{t('debate.countHelp')}</p>
      <div className="grid gap-3 sm:grid-cols-2">
        {seats.map((seat, i) => (
          <SeatField key={i} index={i} seat={seat} known={known} onChange={next => setSeats(list => list.map((s, j) => (j === i ? next : s)))} />
        ))}
      </div>
      <CaptureOptions status={status} value={capture} onChange={setCapture} />
      <Button onClick={start} disabled={starting || seats.some(s => !s.name.trim()) || (!capture.mic && !capture.system) || !debateReady(status)}>
        {starting ? <ThinkingOrb variant="working" size={16} /> : <Swords className="h-4 w-4" />}
        {t(starting ? 'live.starting' : 'debate.start')}
      </Button>
    </div>
  );
}

function DebateHome() {
  const { t } = useI18n();
  const router = useRouter();
  const [status, setStatus] = useState<LiveStatus | null>(null);
  const [debates, setDebates] = useState<LiveSessionSummary[] | null>(null);
  const [known, setKnown] = useState<KnownPerson[] | null>(null);

  useEffect(() => {
    liveService.debates().then(setDebates).catch(e => toast.error(String(e)));
    liveService.debatePeople().then(setKnown).catch(e => toast.error(String(e)));
  }, []);
  const onSettings = useCallback((status: LiveStatus) => setStatus(status), []);

  const running = status?.state.phase === 'recording' ? status.state.sessionId : null;
  const ready = !!status && debateReady(status);
  return (
    <div>
      <PageHeader title={t('nav.debate')} />
      <div className="mx-auto max-w-3xl space-y-6 px-4 py-6 sm:px-8">
        <p className="text-sm text-gray-600">{t('debate.intro')}</p>
        {running && (
          <div className="flex items-center gap-3 rounded-lg border border-red-200 bg-red-50 p-4">
            <span className="h-2.5 w-2.5 animate-pulse rounded-full bg-red-500" />
            <span className="flex-1 text-sm text-red-900">{t('live.running')}</span>
            <Button size="sm" variant="outline" onClick={() => router.push(sessionHref(running, !status?.state.investigationId))}>
              {t('live.open')}
            </Button>
          </div>
        )}
        <section className={`space-y-4 rounded-lg border border-gray-200 bg-white p-6 shadow-sm ${ready ? 'hidden' : ''}`}>
          <div>
            <h3 className="text-lg font-semibold text-gray-900">{t('debate.setupTitle')}</h3>
            <p className="text-sm text-gray-600">{t('debate.setupHelp')}</p>
          </div>
          <LiveSettings onChange={onSettings} />
        </section>
        {status && ready && !running && known && (
          <section className="space-y-4 rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
            <h3 className="flex items-center gap-2 text-lg font-semibold text-gray-900">
              <Swords className="h-5 w-5" /> {t('debate.new')}
            </h3>
            {!status.chatModel && <p className="text-xs text-amber-700">{t('live.notice.noModel')}</p>}
            <NewDebate status={status} known={known} />
          </section>
        )}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold uppercase tracking-wide text-gray-500">{t('debate.past')}</h3>
          {debates === null ? (
            <ThinkingOrb variant="working" size={20} />
          ) : debates.length === 0 ? (
            <p className="text-sm text-gray-500">{t('debate.none')}</p>
          ) : (
            <div className="divide-y divide-gray-100 rounded-lg border border-gray-200 bg-white">
              {debates.map(debate => (
                <SessionRow key={debate.id} session={debate} />
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

function DebateView() {
  const id = useSearchParams().get('id');
  return id ? <LiveSession key={id} id={id} /> : <DebateHome />;
}

/** Debates, outside every investigation: each person is checked against their own investigation
 * (one is made for whoever nobody follows yet). `?id=` is a debate going on or over. */
export default function DebatePage() {
  return (
    <Suspense fallback={null}>
      <DebateView />
    </Suspense>
  );
}
