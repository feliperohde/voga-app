'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { ArrowRight, GitCompareArrows, Settings } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { VogaMark } from '@/components/VogaMark';
import { TitleBar } from '@/components/TitleBar';
import { InvestigationWizard } from '@/components/InvestigationWizard';
import { NewInvestigationDialog } from '@/components/NewInvestigationDialog';
import { AiSetup } from '@/components/AiSetup';
import { AddCard, highlightHref, InvestigationCard, PickMark } from '@/components/home/InvestigationCard';
import { NewsCard } from '@/components/home/NewsCard';
import { Scoreboard } from '@/components/home/Scoreboard';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useCollectProgress } from '@/hooks/useCollectProgress';
import { useDeleteInvestigation } from '@/hooks/useDeleteInvestigation';
import { useLiveState } from '@/hooks/useLiveState';
import { useI18n } from '@/i18n';
import { dashboardService, type InvestigationStatus, type PersonScore } from '@/services/dashboardService';
import { feedService, type FeedItem } from '@/services/feedService';
import type { InvestigationKind } from '@/services/investigationService';
import { cn } from '@/lib/utils';

/** How often Home reads the numbers again while the worker reports progress. */
const RELOAD_MS = 5000;
const NEWS = 8;

function Welcome({ onDone }: { onDone: () => void }) {
  const { t } = useI18n();
  return (
    <div className="mx-auto max-w-2xl space-y-6 px-4 py-12 sm:px-6">
      <div className="space-y-3 text-center">
        <div className="flex justify-center">
          <VogaMark size={64} />
        </div>
        <h1 className="text-3xl font-bold text-gray-900">{t('home.title')}</h1>
        <p className="text-gray-600">{t('home.intro')}</p>
      </div>
      <AiSetup />
      <div className="rounded-lg border border-gray-200 bg-white p-5 shadow-sm sm:p-6">
        <InvestigationWizard onDone={onDone} />
      </div>
    </div>
  );
}

function Section({ title, help, action, children }: { title: string; help: string; action?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="space-y-4">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <div>
          <h2 className="text-xl font-semibold text-gray-900">{title}</h2>
          <p className="text-sm text-gray-500">{help}</p>
        </div>
        {action}
      </div>
      {children}
    </section>
  );
}

function Dashboard() {
  const { t } = useI18n();
  const router = useRouter();
  const { open } = useInvestigation();
  const progress = useCollectProgress();
  const live = useLiveState();
  const remove = useDeleteInvestigation();
  const [statuses, setStatuses] = useState<InvestigationStatus[] | null>(null);
  const [news, setNews] = useState<FeedItem[]>([]);
  const [scores, setScores] = useState<PersonScore[]>([]);
  const [creating, setCreating] = useState<InvestigationKind | null>(null);
  // Picking investigations and subjects to open together (null: not picking)
  const [picked, setPicked] = useState<string[] | null>(null);
  const loadedAt = useRef(0);

  const load = useCallback(async () => {
    loadedAt.current = Date.now();
    const [statuses, news, scores] = await Promise.all([
      dashboardService.load(),
      feedService.list({ investigationIds: null, investigationKind: 'interest', sort: 'date', limit: NEWS }),
      dashboardService.scoreboard(),
    ]);
    setStatuses(statuses);
    setNews(news.items);
    setScores(scores);
  }, []);

  useEffect(() => {
    load().catch(error => console.error('Failed to load Home:', error));
  }, [load]);
  // The worker's progress moves the numbers: read them again, at most every few seconds
  useEffect(() => {
    if (!progress) return;
    const timer = setTimeout(() => load().catch(() => {}), Math.max(0, RELOAD_MS - (Date.now() - loadedAt.current)));
    return () => clearTimeout(timer);
  }, [progress, load]);

  const openInvestigation = (id: string, href = '/investigation') => {
    open({ kind: 'investigation', ids: [id] });
    router.push(href);
  };
  const openJournal = (href = '/journal') => {
    open({ kind: 'journal' });
    router.push(href);
  };
  const pick = (id: string) => setPicked(previous => previous && (previous.includes(id) ? previous.filter(p => p !== id) : [...previous, id]));
  const compare = () => {
    if (!picked || picked.length < 2) return;
    open({ kind: 'investigation', ids: picked });
    router.push('/compare');
  };
  const compareButton = (
    <Button variant={picked ? 'secondary' : 'ghost'} size="sm" onClick={() => setPicked(picked ? null : [])} aria-pressed={!!picked}>
      <GitCompareArrows className="mr-2 h-4 w-4" /> {t('compare.pick')}
    </Button>
  );
  const deleteInvestigation = async (status: InvestigationStatus) => {
    if (await remove({ id: status.id, name: status.name, kind: status.kind, documents: status.documents })) await load();
  };
  const openNews = (item: FeedItem) => {
    feedService.markRead([item.id]).catch(() => {});
    openJournal(`/document?id=${encodeURIComponent(item.id)}`);
  };

  const politics = statuses?.filter(s => s.kind === 'politics') ?? [];
  const interests = statuses?.filter(s => s.kind === 'interest') ?? [];
  const run = (id: string) => progress?.runs.find(r => r.investigationId === id);

  return (
    <div>
      <header className="sticky top-0 z-10 border-b border-gray-200 bg-white">
        <TitleBar className="pr-4 sm:pr-8">
          <div className="flex min-w-0 items-center gap-2">
            {live?.phase === 'recording' && (
              <Button variant="outline" size="sm" onClick={() => (live.investigationId ? openInvestigation(live.investigationId, '/live') : router.push('/debate'))}>
                <span className="mr-2 h-2 w-2 animate-pulse rounded-full bg-red-500" />
                {t('home.recording')}
              </Button>
            )}
            {progress?.running && (
              <span className="hidden min-w-0 items-center gap-2 text-xs text-gray-600 sm:flex" title={progress.current ?? undefined}>
                <ThinkingOrb variant="background" size={16} />
                <span className="truncate">{progress.analyzing ? t('collect.analyzing') : progress.queued > 0 ? t('collect.runningQueued', { count: progress.queued }) : t('collect.running')}</span>
              </span>
            )}
            <Button variant="ghost" size="icon" onClick={() => router.push('/settings')} title={t('nav.settings')} aria-label={t('nav.settings')}>
              <Settings className="h-5 w-5" />
            </Button>
          </div>
        </TitleBar>
      </header>

      <div className="mx-auto max-w-6xl space-y-10 px-4 py-8 sm:px-8">
        <AiSetup />

        {statuses === null ? (
          <div className="flex justify-center py-12">
            <ThinkingOrb variant="working" size={28} />
          </div>
        ) : (
          <>
            <Section title={t('home.investigations')} help={t('home.investigationsHelp')} action={(politics.length > 0 || interests.length > 0) && compareButton}>
              <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
                {politics.map(status => (
                  <InvestigationCard
                    key={status.id}
                    status={status}
                    run={run(status.id)}
                    picked={picked ? picked.includes(status.id) : undefined}
                    onOpen={() => (picked ? pick(status.id) : openInvestigation(status.id))}
                    onHighlight={highlight => openInvestigation(status.id, highlightHref(highlight))}
                    onDelete={() => deleteInvestigation(status).catch(() => {})}
                  />
                ))}
                <AddCard label={t('investigation.new')} help={t('home.newInvestigationHelp')} onClick={() => setCreating('politics')} />
              </div>
            </Section>

            <Section
              title={t('home.journal')}
              help={t('home.journalHelp')}
              action={
                interests.length > 0 && (
                  <div className="flex gap-1">
                    {compareButton}
                    <Button variant="ghost" size="sm" onClick={() => openJournal()}>
                      {t('home.allNews')} <ArrowRight className="ml-1 h-4 w-4" />
                    </Button>
                  </div>
                )
              }
            >
              {interests.length > 0 && (
                <div className="flex flex-wrap gap-2">
                  {interests.map(interest => {
                    const collecting = run(interest.id);
                    return (
                      <button
                        key={interest.id}
                        onClick={() => (picked ? pick(interest.id) : openJournal(`/journal?id=${encodeURIComponent(interest.id)}`))}
                        aria-pressed={picked ? picked.includes(interest.id) : undefined}
                        className={cn(
                          'flex items-center gap-2 rounded-full border bg-white px-3 py-1.5 text-sm shadow-sm hover:bg-gray-50',
                          picked?.includes(interest.id) ? 'border-gray-900 ring-1 ring-gray-900' : 'border-gray-200 hover:border-gray-300',
                        )}
                      >
                        {picked ? (
                          <PickMark picked={picked.includes(interest.id)} />
                        ) : collecting && collecting.done < collecting.total ? (
                          <ThinkingOrb variant="background" size={12} />
                        ) : (
                          <span className={cn('h-2 w-2 rounded-full', interest.failing > 0 ? 'bg-red-500' : 'bg-emerald-500')} />
                        )}
                        <span className="max-w-[16rem] truncate font-medium text-gray-900">{interest.name}</span>
                        {interest.unread > 0 && <span className="rounded-full bg-blue-50 px-1.5 text-xs font-medium tabular-nums text-blue-700">{interest.unread}</span>}
                      </button>
                    );
                  })}
                </div>
              )}
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
                {news.map((item, i) => (
                  <NewsCard key={item.id} item={item} big={i === 0} onOpen={() => openNews(item)} />
                ))}
                <AddCard label={t('interest.new')} help={t('home.newInterestHelp')} onClick={() => setCreating('interest')} />
              </div>
            </Section>

            {scores.length > 0 && (
              <Section title={t('score.title')} help={t('score.help')}>
                <Scoreboard scores={scores} onOpen={(score, href) => openInvestigation(score.investigationId, href)} />
              </Section>
            )}
          </>
        )}
      </div>
      {picked && (
        <div className="sticky bottom-4 z-20 mx-auto flex w-fit items-center gap-3 rounded-full border border-gray-200 bg-white px-4 py-2 shadow-lg">
          <span className="text-sm text-gray-700">{picked.length < 2 ? t('compare.pickHelp') : t('compare.picked', { count: picked.length })}</span>
          <Button size="sm" onClick={compare} disabled={picked.length < 2}>
            {t('compare.open.button')}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setPicked(null)}>
            {t('common.cancel')}
          </Button>
        </div>
      )}
      <NewInvestigationDialog
        open={creating !== null}
        kind={creating ?? undefined}
        onOpenChange={next => {
          if (next) return;
          setCreating(null);
          load().catch(() => {});
        }}
      />
    </div>
  );
}

export default function Home() {
  const { investigations, loaded, open } = useInvestigation();
  // Home closes what was open: the sidebar comes back with the next card opened
  useEffect(() => {
    if (loaded) open(null);
  }, [loaded, open]);
  // The wizard creates the investigation halfway: it stays until its last step
  const [wizard, setWizard] = useState(false);
  useEffect(() => {
    if (loaded && investigations.length === 0) setWizard(true);
  }, [loaded, investigations.length]);
  if (!loaded) {
    return (
      <div className="flex min-h-[60vh] items-center justify-center">
        <ThinkingOrb variant="working" size={28} />
      </div>
    );
  }
  return wizard || investigations.length === 0 ? <Welcome onDone={() => setWizard(false)} /> : <Dashboard />;
}
