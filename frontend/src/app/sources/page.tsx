'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import {
  BookOpen,
  Building2,
  HandCoins,
  MessageCircle,
  Globe,
  Hash,
  Landmark,
  Link2,
  Link2Off,
  Newspaper,
  Pause,
  Pencil,
  Play,
  PlaySquare,
  Plus,
  Rss,
  ScanSearch,
  Search,
  Trash2,
  User,
  Vote,
  Youtube,
  type LucideIcon,
} from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { useConfirm } from '@/components/ui/confirm-dialog';
import { PageHeader } from '@/components/PageHeader';
import { OfficialDialog } from '@/components/OfficialDialog';
import { DiscoverySettings } from '@/components/DiscoverySettings';
import { ScheduleSettings } from '@/components/ScheduleSettings';
import { SourceSuggestions } from '@/components/SourceSuggestions';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useCollectProgress } from '@/hooks/useCollectProgress';
import { useDeleteInvestigation } from '@/hooks/useDeleteInvestigation';
import { every } from '@/lib/every';
import { formatDate, useI18n, type MessageKey } from '@/i18n';
import { investigationService, InvestigationKind, Source, SourceKind, Target, TargetKind } from '@/services/investigationService';

const SOURCE_KINDS: { kind: SourceKind; icon: LucideIcon; label: MessageKey; placeholder: MessageKey }[] = [
  { kind: 'rss', icon: Rss, label: 'sources.kind.rss', placeholder: 'sources.placeholder.rss' },
  { kind: 'page', icon: Globe, label: 'sources.kind.page', placeholder: 'sources.placeholder.page' },
  { kind: 'youtube_channel', icon: Youtube, label: 'sources.kind.youtube_channel', placeholder: 'sources.placeholder.youtube_channel' },
  { kind: 'youtube_video', icon: PlaySquare, label: 'sources.kind.youtube_video', placeholder: 'sources.placeholder.youtube_video' },
  { kind: 'news_search', icon: Newspaper, label: 'sources.kind.news_search', placeholder: 'sources.placeholder.search' },
  { kind: 'youtube_search', icon: Search, label: 'sources.kind.youtube_search', placeholder: 'sources.placeholder.search' },
  { kind: 'web_search', icon: ScanSearch, label: 'sources.kind.web_search', placeholder: 'sources.placeholder.search' },
  { kind: 'gazette_search', icon: BookOpen, label: 'sources.kind.gazette_search', placeholder: 'sources.placeholder.search' },
  { kind: 'social', icon: MessageCircle, label: 'sources.kind.social', placeholder: 'sources.placeholder.social' },
];
/** Made by linking a target to its official records (not added by hand). */
const OFFICIAL_KINDS: { kind: SourceKind; icon: LucideIcon; label: MessageKey; placeholder: MessageKey }[] = [
  { kind: 'camara_deputado', icon: Landmark, label: 'sources.kind.camara_deputado', placeholder: 'sources.placeholder.search' },
  { kind: 'senado_senador', icon: Landmark, label: 'sources.kind.senado_senador', placeholder: 'sources.placeholder.search' },
  { kind: 'tse_candidato', icon: Vote, label: 'sources.kind.tse_candidato', placeholder: 'sources.placeholder.search' },
  { kind: 'transparencia_emendas', icon: HandCoins, label: 'sources.kind.transparencia_emendas', placeholder: 'sources.placeholder.search' },
  { kind: 'alesp_deputado', icon: Landmark, label: 'sources.kind.alesp_deputado', placeholder: 'sources.placeholder.search' },
  { kind: 'pncp_prefeitura', icon: Building2, label: 'sources.kind.pncp_prefeitura', placeholder: 'sources.placeholder.search' },
  { kind: 'emendas_estaduais', icon: HandCoins, label: 'sources.kind.emendas_estaduais', placeholder: 'sources.placeholder.search' },
];
const KIND = Object.fromEntries([...SOURCE_KINDS, ...OFFICIAL_KINDS].map(k => [k.kind, k])) as Record<SourceKind, (typeof SOURCE_KINDS)[number]>;

const TARGET_KINDS: { kind: TargetKind; icon: LucideIcon; label: MessageKey }[] = [
  { kind: 'person', icon: User, label: 'targets.kind.person' },
  { kind: 'organization', icon: Building2, label: 'targets.kind.organization' },
  { kind: 'topic', icon: Hash, label: 'targets.kind.topic' },
];

function Section({ title, help, children }: { title: string; help: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-gray-200 bg-white shadow-sm">
      <div className="border-b border-gray-100 px-5 py-4">
        <h2 className="font-semibold text-gray-900">{title}</h2>
        <p className="text-sm text-gray-600">{help}</p>
      </div>
      {children}
    </section>
  );
}

function TargetDialog({
  target,
  investigationId,
  interest,
  onClose,
  onSaved,
}: {
  /** null: closed; 'new': a new target. */
  target: Target | 'new' | null;
  investigationId: string;
  /** An interest's targets are themes by default. */
  interest: boolean;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useI18n();
  const editing = target && target !== 'new' ? target : null;
  const [kind, setKind] = useState<TargetKind>('person');
  const [name, setName] = useState('');
  const [aliases, setAliases] = useState('');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setKind(editing?.kind ?? (interest ? 'topic' : 'person'));
    setName(editing?.name ?? '');
    setAliases(editing?.aliases.join(', ') ?? '');
  }, [target, editing, interest]);

  const save = async () => {
    setSaving(true);
    try {
      await investigationService.saveTarget({
        id: editing?.id ?? null,
        investigationId,
        kind,
        name,
        aliases: aliases.split(',').map(a => a.trim()).filter(Boolean),
      });
      onSaved();
      onClose();
    } catch (error) {
      toast.error(t('common.saveFailed'), { description: String(error) });
    } finally {
      setSaving(false);
    }
  };

  // An interest's theme is searched as loose words; its other terms only steer the Feed's relevance
  const loose = interest && kind === 'topic';

  return (
    <Dialog open={target !== null} onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{editing ? t('targets.edit') : t(interest ? 'targets.add.interest' : 'targets.add')}</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label>{t('targets.kindLabel')}</Label>
            <SelectField value={kind} onValueChange={value => setKind(value as TargetKind)} options={TARGET_KINDS.map(k => ({ value: k.kind, label: t(k.label) }))} />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="target-name">{t('targets.name')}</Label>
            <Input id="target-name" value={name} onChange={e => setName(e.target.value)} autoFocus />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="target-aliases">{t(loose ? 'targets.terms' : 'targets.aliases')}</Label>
            <Input id="target-aliases" value={aliases} onChange={e => setAliases(e.target.value)} placeholder={t(loose ? 'targets.termsPlaceholder' : 'targets.aliasesPlaceholder')} />
            <p className="text-xs text-gray-500">{t(loose ? 'targets.termsHelp' : 'targets.aliasesHelp')}</p>
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>
              {t('common.cancel')}
            </Button>
            <Button onClick={save} disabled={saving || !name.trim()}>
              {saving && <ThinkingOrb variant="working" size={16} className="mr-2" />}
              {t('common.save')}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function SourceRow({ source, target, onChanged }: { source: Source; target: Target | undefined; onChanged: () => void }) {
  const { t, locale } = useI18n();
  const confirm = useConfirm();
  const kind = KIND[source.kind];
  const Icon = kind.icon;
  const isSearch = source.kind.endsWith('_search');
  // A site or feed: what it brings can have its links followed
  const canFollow = source.kind === 'page' || source.kind === 'rss';
  const isOfficial = OFFICIAL_KINDS.some(k => k.kind === source.kind);

  const act = (action: () => Promise<void>) => action().then(onChanged).catch(error => toast.error(String(error)));
  const remove = async () => {
    if (await confirm({ title: t('sources.deleteTitle'), description: t(isOfficial ? 'sources.deleteOfficialHelp' : 'sources.deleteHelp'), destructive: true, confirmLabel: t('common.delete') })) {
      act(() => investigationService.deleteSource(source.id));
    }
  };

  const details = [
    t(kind.label),
    target && t('sources.ofTarget', { name: target.name }),
    source.origin === 'llm' && t('sources.byAi'),
    canFollow && source.followLinks && t('sources.followsLinks'),
    source.status === 'paused' ? t('sources.paused') : source.status === 'done' ? t('sources.done') : every(t, source.everyMinutes),
    source.lastRunAt && t('sources.lastRun', { date: formatDate(locale, source.lastRunAt, { dateStyle: 'short', timeStyle: 'short' }) }),
    t('sources.documents', { count: source.documentCount }),
    source.queued > 0 && t('sources.queued', { count: source.queued }),
  ].filter(Boolean);

  return (
    <div className="flex items-start gap-3 px-5 py-3">
      <Icon className="mt-0.5 h-4 w-4 flex-shrink-0 text-gray-500" />
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium text-gray-900" title={source.url}>
          {isSearch ? `${t(kind.label)}: ${source.url}` : isOfficial ? `${t(kind.label)}: ${source.title?.split(' · ')[0] ?? source.url}` : (source.title ?? source.url)}
        </div>
        <div className="truncate text-xs text-gray-500">{details.join(' · ')}</div>
        {source.note && <div className="text-xs text-gray-500">{source.note}</div>}
        {source.lastError && <div className="mt-1 text-xs text-red-600">{source.lastError}</div>}
      </div>
      <div className="flex flex-shrink-0 gap-1">
        {canFollow && (
          <Button
            variant="ghost"
            size="icon"
            title={source.followLinks ? t('sources.stopFollowing') : t('sources.follow')}
            onClick={() => act(() => investigationService.updateSource(source.id, { followLinks: !source.followLinks }))}
          >
            {source.followLinks ? <Link2 className="h-4 w-4 text-blue-700" /> : <Link2Off className="h-4 w-4" />}
          </Button>
        )}
        <Button variant="ghost" size="icon" title={t('sources.runNow')} onClick={() => act(() => investigationService.runSourceNow(source.id))}>
          <Play className="h-4 w-4" />
        </Button>
        {source.status !== 'done' && (
          <Button
            variant="ghost"
            size="icon"
            title={source.status === 'paused' ? t('sources.resume') : t('sources.pause')}
            onClick={() => act(() => investigationService.updateSource(source.id, { status: source.status === 'paused' ? 'active' : 'paused' }))}
          >
            {source.status === 'paused' ? <Play className="h-4 w-4 text-green-700" /> : <Pause className="h-4 w-4" />}
          </Button>
        )}
        <Button variant="ghost" size="icon" title={t('common.delete')} onClick={remove}>
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function AddSource({ investigationId, onAdded }: { investigationId: string; onAdded: () => void }) {
  const { t } = useI18n();
  const [kind, setKind] = useState<SourceKind>('rss');
  const [url, setUrl] = useState('');
  const [adding, setAdding] = useState(false);
  const add = async () => {
    setAdding(true);
    try {
      await investigationService.addSource(investigationId, kind, url);
      setUrl('');
      onAdded();
      toast.success(t('sources.added'));
    } catch (error) {
      toast.error(t('sources.addFailed'), { description: String(error) });
    } finally {
      setAdding(false);
    }
  };
  return (
    <div className="flex flex-wrap items-end gap-2 border-t border-gray-100 px-5 py-4">
      <div className="w-52">
        <SelectField value={kind} onValueChange={value => setKind(value as SourceKind)} options={SOURCE_KINDS.map(k => ({ value: k.kind, label: t(k.label) }))} />
      </div>
      <Input className="min-w-[16rem] flex-1" value={url} onChange={e => setUrl(e.target.value)} onKeyDown={e => e.key === 'Enter' && add()} placeholder={t(KIND[kind].placeholder)} />
      <Button onClick={add} disabled={adding || !url.trim()}>
        {adding ? <ThinkingOrb variant="working" size={16} className="mr-2" /> : <Plus className="mr-2 h-4 w-4" />}
        {t('sources.add')}
      </Button>
    </div>
  );
}

export default function SourcesPage() {
  const { t } = useI18n();
  const router = useRouter();
  const confirm = useConfirm();
  const remove = useDeleteInvestigation();
  const { current, refresh } = useInvestigation();
  const progress = useCollectProgress();
  const [targets, setTargets] = useState<Target[]>([]);
  const [sources, setSources] = useState<Source[] | null>(null);
  const [editing, setEditing] = useState<Target | 'new' | null>(null);
  const [linking, setLinking] = useState<Target | null>(null);

  const load = useCallback(async () => {
    if (!current) return;
    const [targets, sources] = await Promise.all([investigationService.targets(current.id), investigationService.sources(current.id)]);
    setTargets(targets);
    setSources(sources);
  }, [current]);

  useEffect(() => {
    load().catch(error => console.error('Failed to load sources:', error));
  }, [load]);
  // Errors, counts and last runs change while collecting
  useEffect(() => {
    if (progress && !progress.running) load().catch(() => {});
  }, [progress?.running, progress, load]);

  const changed = () => {
    load().catch(() => {});
    refresh().catch(() => {});
  };

  const removeTarget = async (target: Target) => {
    if (await confirm({ title: t('targets.deleteTitle', { name: target.name }), description: t('targets.deleteHelp'), destructive: true, confirmLabel: t('common.delete') })) {
      investigationService.deleteTarget(target.id).then(changed).catch(error => toast.error(String(error)));
    }
  };

  const changeKind = async (kind: InvestigationKind) => {
    if (!current || kind === current.kind) return;
    const ok = await confirm({
      title: t(kind === 'interest' ? 'investigation.toInterest' : 'investigation.toPolitics'),
      description: t(kind === 'interest' ? 'investigation.toInterestHelp' : 'investigation.toPoliticsHelp'),
      confirmLabel: t('investigation.changeKind'),
    });
    if (!ok) return;
    investigationService
      .setKind(current.id, kind)
      .then(changed)
      .catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));
  };

  if (!current) return <PageHeader title={t('nav.sources')} />;
  const targetById = new Map(targets.map(target => [target.id, target]));
  const interest = current.kind === 'interest';
  const suggested = (sources ?? []).filter(s => s.status === 'suggested');
  const listed = (sources ?? []).filter(s => s.status !== 'suggested' && s.status !== 'rejected');

  return (
    <div>
      <PageHeader title={t('nav.sources')} />
      <div className="mx-auto max-w-4xl space-y-6 px-4 py-6 sm:px-8">
        <SourceSuggestions sources={suggested} onChanged={changed} />
        <section className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-lg border border-gray-200 bg-white px-5 py-4 shadow-sm">
          <div className="min-w-0 flex-1">
            <h2 className="font-semibold text-gray-900">{t('investigation.kind')}</h2>
            <p className="text-sm text-gray-600">{t(interest ? 'investigation.kind.interestHelp' : 'investigation.kind.politicsHelp')}</p>
          </div>
          <div className="w-44">
            <SelectField
              value={current.kind}
              onValueChange={value => changeKind(value as InvestigationKind)}
              aria-label={t('investigation.kind')}
              options={[
                { value: 'politics', label: t('investigation.kind.politics') },
                { value: 'interest', label: t('investigation.kind.interest') },
              ]}
            />
          </div>
        </section>
        <Section title={t(interest ? 'targets.title.interest' : 'targets.title')} help={t(interest ? 'targets.help.interest' : 'targets.help')}>
          <div className="divide-y divide-gray-100">
            {targets.length === 0 && <p className="px-5 py-4 text-sm text-gray-500">{t(interest ? 'targets.empty.interest' : 'targets.empty')}</p>}
            {targets.map(target => {
              const Icon = TARGET_KINDS.find(k => k.kind === target.kind)?.icon ?? User;
              return (
                <div key={target.id} className="flex items-center gap-3 px-5 py-3">
                  <Icon className="h-4 w-4 flex-shrink-0 text-gray-500" />
                  <div className="min-w-0 flex-1">
                    <div className="font-medium text-gray-900">{target.name}</div>
                    {target.aliases.length > 0 && <div className="truncate text-xs text-gray-500">{target.aliases.join(', ')}</div>}
                    {target.official.length > 0 && (
                      <div className="mt-1 flex flex-wrap gap-1">
                        {target.official.map(o => (
                          <span key={`${o.kind}:${o.id}`} title={o.detail} className="rounded bg-blue-50 px-1.5 py-0.5 text-[11px] font-medium text-blue-800">
                            {t(`sources.kind.${o.kind}`)}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                  {target.kind === 'person' && !interest && (
                    <Button variant="ghost" size="sm" title={t('official.button')} onClick={() => setLinking(target)}>
                      <Landmark className="mr-1.5 h-4 w-4" /> {t('official.button')}
                    </Button>
                  )}
                  <Button variant="ghost" size="icon" title={t('targets.edit')} onClick={() => setEditing(target)}>
                    <Pencil className="h-4 w-4" />
                  </Button>
                  <Button variant="ghost" size="icon" title={t('common.delete')} onClick={() => removeTarget(target)}>
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              );
            })}
          </div>
          <div className="border-t border-gray-100 px-5 py-3">
            <Button variant="outline" size="sm" onClick={() => setEditing('new')}>
              <Plus className="mr-2 h-4 w-4" /> {t(interest ? 'targets.add.interest' : 'targets.add')}
            </Button>
          </div>
        </Section>

        <Section title={t('sources.title')} help={t(interest ? 'sources.help.interest' : 'sources.help')}>
          {sources === null ? (
            <div className="flex justify-center p-6">
              <ThinkingOrb variant="working" size={20} />
            </div>
          ) : (
            <div className="divide-y divide-gray-100">
              {listed.length === 0 && <p className="px-5 py-4 text-sm text-gray-500">{t('sources.empty')}</p>}
              {listed.map(source => (
                <SourceRow key={source.id} source={source} target={source.targetId ? targetById.get(source.targetId) : undefined} onChanged={changed} />
              ))}
            </div>
          )}
          <AddSource investigationId={current.id} onAdded={changed} />
        </Section>

        <Section title={t('schedule.title')} help={t('schedule.help')}>
          <ScheduleSettings investigationId={current.id} interest={interest} onSaved={changed} />
        </Section>

        <Section title={t('discovery.title')} help={t(interest ? 'discovery.help.interest' : 'discovery.help')}>
          <DiscoverySettings investigationId={current.id} onSuggested={changed} />
        </Section>

        <section className="flex flex-wrap items-center gap-x-4 gap-y-3 rounded-lg border border-red-200 bg-white px-5 py-4 shadow-sm">
          <div className="min-w-0 flex-1">
            <h2 className="font-semibold text-gray-900">{t(interest ? 'interest.delete' : 'investigation.delete')}</h2>
            <p className="text-sm text-gray-600">{t(interest ? 'interest.deleteSectionHelp' : 'investigation.deleteSectionHelp')}</p>
          </div>
          <Button
            variant="outline"
            className="border-red-200 text-red-700 hover:bg-red-50 hover:text-red-800"
            onClick={() =>
              remove({ id: current.id, name: current.name, kind: current.kind, documents: current.documentCount })
                .then(deleted => deleted && router.push('/'))
                .catch(() => {})
            }
          >
            <Trash2 className="mr-2 h-4 w-4" /> {t('common.delete')}
          </Button>
        </section>
      </div>
      <TargetDialog target={editing} investigationId={current.id} interest={interest} onClose={() => setEditing(null)} onSaved={changed} />
      <OfficialDialog target={linking} onClose={() => setLinking(null)} onLinked={changed} />
    </div>
  );
}
