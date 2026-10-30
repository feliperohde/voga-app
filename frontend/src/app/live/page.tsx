'use client';

import { Suspense, useCallback, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { Mic, Radio } from 'lucide-react';
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
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useI18n } from '@/i18n';
import { investigationService, Target } from '@/services/investigationService';
import { LiveSessionSummary, liveService, LiveStatus, sessionHref, transcriptionReady } from '@/services/liveService';

function NewSession({ status, investigationId, people }: { status: LiveStatus; investigationId: string; people: Target[] }) {
  const { t } = useI18n();
  const router = useRouter();
  const [title, setTitle] = useState('');
  const [targetId, setTargetId] = useState(people[0]?.id ?? '');
  const [capture, setCapture] = useState<Capture>(() => initialCapture(status));
  const [starting, setStarting] = useState(false);

  const start = () => {
    setStarting(true);
    liveService
      .start({
        investigationId,
        targetId,
        title: title.trim() || t('live.defaultTitle', { name: people.find(p => p.id === targetId)?.name ?? '' }),
        microphone: microphoneOf(capture),
        systemAudio: capture.system,
        autoCheck: capture.autoCheck,
      })
      .then(id => router.push(sessionHref(id, false)))
      .catch(e => toast.error(t('live.startFailed'), { description: String(e) }))
      .finally(() => setStarting(false));
  };

  if (people.length === 0) return <p className="text-sm text-gray-600">{t('live.noPeople')}</p>;
  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="live-person">{t('live.person')}</Label>
          <SelectField id="live-person" value={targetId} onValueChange={setTargetId} options={people.map(p => ({ value: p.id, label: p.name }))} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="live-title">{t('live.sessionTitle')}</Label>
          <Input id="live-title" value={title} onChange={e => setTitle(e.target.value)} placeholder={t('live.sessionTitlePlaceholder')} />
        </div>
      </div>
      <CaptureOptions status={status} value={capture} onChange={setCapture} />
      <Button onClick={start} disabled={starting || !targetId || (!capture.mic && !capture.system) || !transcriptionReady(status)}>
        {starting ? <ThinkingOrb variant="working" size={16} /> : <Mic className="h-4 w-4" />}
        {t(starting ? 'live.starting' : 'live.start')}
      </Button>
    </div>
  );
}

function LiveHome() {
  const { t } = useI18n();
  const router = useRouter();
  const { current } = useInvestigation();
  const [status, setStatus] = useState<LiveStatus | null>(null);
  const [sessions, setSessions] = useState<LiveSessionSummary[] | null>(null);
  const [people, setPeople] = useState<Target[] | null>(null);

  useEffect(() => {
    if (!current) return;
    liveService.sessions(current.id).then(setSessions).catch(e => toast.error(String(e)));
    investigationService
      .targets(current.id)
      .then(targets => setPeople(targets.filter(target => target.kind === 'person')))
      .catch(e => toast.error(String(e)));
  }, [current]);
  const onSettings = useCallback((status: LiveStatus) => setStatus(status), []);

  const running = status?.state.phase === 'recording' ? status.state.sessionId : null;
  const runningDebate = !status?.state.investigationId;
  return (
    <div>
      <PageHeader title={t('nav.live')} />
      <div className="mx-auto max-w-3xl space-y-6 px-4 py-6 sm:px-8">
        <p className="text-sm text-gray-600">{t('live.intro')}</p>
        {running && (
          <div className="flex items-center gap-3 rounded-lg border border-red-200 bg-red-50 p-4">
            <span className="h-2.5 w-2.5 animate-pulse rounded-full bg-red-500" />
            <span className="flex-1 text-sm text-red-900">{t('live.running')}</span>
            <Button size="sm" variant="outline" onClick={() => router.push(sessionHref(running, runningDebate))}>
              {t('live.open')}
            </Button>
          </div>
        )}
        <section className={`space-y-4 rounded-lg border border-gray-200 bg-white p-6 shadow-sm ${status && transcriptionReady(status) ? 'hidden' : ''}`}>
          <div>
            <h3 className="text-lg font-semibold text-gray-900">{t('live.setupTitle')}</h3>
            <p className="text-sm text-gray-600">{t('live.setupHelp')}</p>
          </div>
          <LiveSettings onChange={onSettings} />
        </section>
        {status && transcriptionReady(status) && !running && current && people && (
          <section className="space-y-4 rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
            <h3 className="flex items-center gap-2 text-lg font-semibold text-gray-900">
              <Radio className="h-5 w-5" /> {t('live.newSession')}
            </h3>
            {!status.chatModel && <p className="text-xs text-amber-700">{t('live.notice.noModel')}</p>}
            <NewSession status={status} investigationId={current.id} people={people} />
          </section>
        )}
        <section className="space-y-2">
          <h3 className="text-sm font-semibold uppercase tracking-wide text-gray-500">{t('live.past')}</h3>
          {sessions === null ? (
            <ThinkingOrb variant="working" size={20} />
          ) : sessions.length === 0 ? (
            <p className="text-sm text-gray-500">{t('live.noSessions')}</p>
          ) : (
            <div className="divide-y divide-gray-100 rounded-lg border border-gray-200 bg-white">
              {sessions.map(session => (
                <SessionRow key={session.id} session={session} />
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

function LiveView() {
  const id = useSearchParams().get('id');
  return id ? <LiveSession key={id} id={id} /> : <LiveHome />;
}

export default function LivePage() {
  return (
    <Suspense fallback={null}>
      <LiveView />
    </Suspense>
  );
}
