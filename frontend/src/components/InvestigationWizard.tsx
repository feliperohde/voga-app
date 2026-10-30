'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { BookOpen, Check, Globe, Hash, Landmark, MessageCircle, Newspaper, Plus, Radio, Rss, ScanSearch, Search, Trash2, Youtube, type LucideIcon } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { OfficialPicker } from '@/components/OfficialDialog';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useI18n, type MessageKey } from '@/i18n';
import { Investigation, investigationService, InvestigationKind, Source, SourceKind, Target } from '@/services/investigationService';

type Step = 'kind' | 'subject' | 'official' | 'sources' | 'done';

/** A person or theme typed in the wizard, before it is saved. */
interface Entry {
  name: string;
  aliases: string;
}

const STEP_LABEL: Record<Step, MessageKey> = {
  kind: 'wizard.step.kind',
  subject: 'wizard.step.subject',
  official: 'wizard.step.official',
  sources: 'wizard.step.sources',
  done: 'wizard.step.done',
};

/** The two kinds, each with examples of what people follow with it. */
const KINDS: { kind: InvestigationKind; icon: LucideIcon; examples: MessageKey[] }[] = [
  { kind: 'politics', icon: Landmark, examples: ['wizard.kind.politics.example1', 'wizard.kind.politics.example2', 'wizard.kind.politics.example3'] },
  { kind: 'interest', icon: Hash, examples: ['wizard.kind.interest.example1', 'wizard.kind.interest.example2', 'wizard.kind.interest.example3'] },
];

/** What a person can add by hand, each explained with a real case. */
const EXTRA_SOURCES: { kind: SourceKind; icon: LucideIcon; placeholder: MessageKey }[] = [
  { kind: 'rss', icon: Rss, placeholder: 'sources.placeholder.rss' },
  { kind: 'youtube_channel', icon: Youtube, placeholder: 'sources.placeholder.youtube_channel' },
  { kind: 'social', icon: MessageCircle, placeholder: 'sources.placeholder.social' },
  { kind: 'page', icon: Globe, placeholder: 'sources.placeholder.page' },
];

/** The searches each target gets on its own, which the sources step lists with what they bring. */
const AUTOMATIC: { kind: SourceKind; icon: LucideIcon }[] = [
  { kind: 'news_search', icon: Newspaper },
  { kind: 'youtube_search', icon: Search },
  { kind: 'gazette_search', icon: BookOpen },
  { kind: 'web_search', icon: ScanSearch },
];

const OFFICIAL_KINDS: SourceKind[] = ['camara_deputado', 'senado_senador', 'tse_candidato', 'transparencia_emendas', 'alesp_deputado', 'pncp_prefeitura', 'emendas_estaduais'];

const splitAliases = (aliases: string) =>
  aliases
    .split(',')
    .map(a => a.trim())
    .filter(Boolean);

function Steps({ steps, current }: { steps: Step[]; current: Step }) {
  const { t } = useI18n();
  const index = steps.indexOf(current);
  return (
    <div className="space-y-2">
      <p className="text-xs font-medium text-gray-500">
        {t('wizard.progress', { step: index + 1, total: steps.length })} · {t(STEP_LABEL[current])}
      </p>
      <ol className="flex gap-1.5" aria-label={t('wizard.progressLabel')}>
        {steps.map((step, i) => (
          <li
            key={step}
            aria-current={i === index ? 'step' : undefined}
            title={t(STEP_LABEL[step])}
            className={`h-1.5 flex-1 rounded-full ${i <= index ? 'bg-blue-600' : 'bg-gray-200'}`}
          />
        ))}
      </ol>
    </div>
  );
}

function Heading({ title, help }: { title: string; help: string }) {
  return (
    <div className="space-y-1">
      <h2 className="text-xl font-semibold text-gray-900">{title}</h2>
      <p className="text-sm text-gray-600">{help}</p>
    </div>
  );
}

/** A worked example, as it would happen, so the steps that follow make sense. */
function Example({ title, lines }: { title: string; lines: string[] }) {
  return (
    <div className="rounded-lg border border-gray-200 bg-gray-50 p-4">
      <p className="mb-2 text-sm font-medium text-gray-900">{title}</p>
      <ol className="list-decimal space-y-1.5 pl-5 text-sm text-gray-700 marker:text-gray-400">
        {lines.map(line => (
          <li key={line}>{line}</li>
        ))}
      </ol>
    </div>
  );
}

/**
 * Guides someone through a new investigation: what kind, who or what it follows (with the
 * names the news use), the official records of each person, other sources, and what happens
 * next. The investigation exists from the second step on; the rest can be skipped.
 */
export function InvestigationWizard({ kind: chosen, onDone }: { kind?: InvestigationKind; onDone?: () => void }) {
  const { t } = useI18n();
  const router = useRouter();
  const { refresh, select } = useInvestigation();
  // A kind chosen beforehand (Home's cards) skips its step
  const [step, setStep] = useState<Step>(chosen ? 'subject' : 'kind');
  const [kind, setKind] = useState<InvestigationKind>(chosen ?? 'politics');
  const [entries, setEntries] = useState<Entry[]>([{ name: '', aliases: '' }]);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [saving, setSaving] = useState(false);
  const [created, setCreated] = useState<Investigation | null>(null);
  const [targets, setTargets] = useState<Target[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [extraKind, setExtraKind] = useState<SourceKind>('rss');
  const [extraUrl, setExtraUrl] = useState('');
  const [adding, setAdding] = useState(false);
  const top = useRef<HTMLDivElement>(null);
  const shownStep = useRef(step);

  // Each step starts at its top: the page (or the dialog) stays scrolled where the last one ended
  useEffect(() => {
    if (shownStep.current === step) return;
    shownStep.current = step;
    top.current?.scrollIntoView({ block: 'start' });
  }, [step]);

  const interest = kind === 'interest';
  const people = targets.filter(target => target.kind === 'person');
  // Before the investigation exists, a political one is expected to have someone to link
  const withOfficial = !interest && (!created || people.length > 0);
  const steps: Step[] = [...(chosen ? [] : (['kind'] as const)), 'subject', ...(withOfficial ? (['official'] as const) : []), 'sources', 'done'];
  const next = () => setStep(steps[steps.indexOf(step) + 1]);
  const typed = entries.filter(entry => entry.name.trim());
  const first = typed[0]?.name.trim() ?? '';

  const reload = useCallback(async (investigationId: string) => {
    const [targets, sources] = await Promise.all([investigationService.targets(investigationId), investigationService.sources(investigationId)]);
    setTargets(targets);
    setSources(sources);
  }, []);

  const setEntry = (index: number, change: Partial<Entry>) => setEntries(entries.map((entry, i) => (i === index ? { ...entry, ...change } : entry)));

  const create = async () => {
    const title = name.trim() || first;
    if (!title) return;
    setSaving(true);
    try {
      // After a failure halfway, the investigation and the targets already saved are reused
      const investigation = created ?? (await investigationService.create(title, description, kind));
      setCreated(investigation);
      const saved = new Set((await investigationService.targets(investigation.id)).map(target => target.name));
      for (const entry of typed) {
        if (saved.has(entry.name.trim())) continue;
        await investigationService.saveTarget({
          id: null,
          investigationId: investigation.id,
          kind: interest ? 'topic' : 'person',
          name: entry.name.trim(),
          aliases: splitAliases(entry.aliases),
        });
      }
      await refresh();
      select(investigation.id);
      await reload(investigation.id);
      setStep(!interest && typed.length > 0 ? 'official' : 'sources');
    } catch (error) {
      toast.error(t('investigation.createFailed'), { description: String(error) });
    } finally {
      setSaving(false);
    }
  };

  const addSource = async () => {
    if (!created || !extraUrl.trim()) return;
    setAdding(true);
    try {
      await investigationService.addSource(created.id, extraKind, extraUrl.trim());
      setExtraUrl('');
      await reload(created.id);
      toast.success(t('sources.added'));
    } catch (error) {
      toast.error(t('sources.addFailed'), { description: String(error) });
    } finally {
      setAdding(false);
    }
  };

  const finish = (href: string) => {
    if (created) refresh().catch(() => {});
    onDone?.();
    router.push(href);
  };

  const body = (() => {
    switch (step) {
      case 'kind':
        return (
          <div className="space-y-5">
            <Heading title={t('wizard.kind.title')} help={t('wizard.kind.help')} />
            <div className="grid gap-3 sm:grid-cols-2">
              {KINDS.map(option => {
                const selected = kind === option.kind;
                const Icon = option.icon;
                return (
                  <button
                    key={option.kind}
                    type="button"
                    aria-pressed={selected}
                    onClick={() => setKind(option.kind)}
                    className={`flex flex-col gap-2 rounded-lg border p-4 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-600 ${
                      selected ? 'border-blue-600 bg-blue-50 ring-1 ring-blue-600' : 'border-gray-200 bg-white hover:border-gray-300'
                    }`}
                  >
                    <span className="flex items-center gap-2 font-semibold text-gray-900">
                      <Icon className="h-4 w-4 flex-shrink-0 text-gray-600" />
                      {t(`wizard.kind.${option.kind}.title`)}
                      {selected && <Check className="ml-auto h-4 w-4 text-blue-700" />}
                    </span>
                    <span className="text-sm text-gray-700">{t(`wizard.kind.${option.kind}.help`)}</span>
                    <span className="pt-1 text-xs font-medium text-gray-500">{t('wizard.forExample')}</span>
                    <ul className="list-disc space-y-1 pl-4 text-xs text-gray-600">
                      {option.examples.map(example => (
                        <li key={example}>{t(example)}</li>
                      ))}
                    </ul>
                  </button>
                );
              })}
            </div>
            <Example
              title={t(interest ? 'wizard.case.interest.title' : 'wizard.case.politics.title')}
              lines={
                interest
                  ? [t('wizard.case.interest.1'), t('wizard.case.interest.2'), t('wizard.case.interest.3')]
                  : [t('wizard.case.politics.1'), t('wizard.case.politics.2'), t('wizard.case.politics.3'), t('wizard.case.politics.4')]
              }
            />
            <div className="flex justify-end">
              <Button onClick={next}>{t(interest ? 'wizard.kind.nextInterest' : 'wizard.kind.nextPolitics')}</Button>
            </div>
          </div>
        );

      case 'subject':
        return (
          <div className="space-y-5">
            <Heading title={t(interest ? 'wizard.subject.title.interest' : 'wizard.subject.title')} help={t(interest ? 'wizard.subject.help.interest' : 'wizard.subject.help')} />
            <div className="space-y-3">
              {entries.map((entry, index) => (
                <div key={index} className="space-y-3 rounded-lg border border-gray-200 bg-white p-4">
                  <div className="flex items-end gap-2">
                    <div className="min-w-0 flex-1 space-y-1.5">
                      <Label htmlFor={`wizard-name-${index}`}>{t(interest ? 'wizard.subject.topic' : 'wizard.subject.person')}</Label>
                      <Input
                        id={`wizard-name-${index}`}
                        value={entry.name}
                        onChange={e => setEntry(index, { name: e.target.value })}
                        placeholder={t(interest ? 'investigation.firstTopicPlaceholder' : 'wizard.subject.personPlaceholder')}
                        autoFocus={index === 0}
                      />
                    </div>
                    {entries.length > 1 && (
                      <Button variant="ghost" size="icon" title={t('common.delete')} aria-label={t('common.delete')} onClick={() => setEntries(entries.filter((_, i) => i !== index))}>
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    )}
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor={`wizard-aliases-${index}`}>{t(interest ? 'targets.terms' : 'targets.aliases')}</Label>
                    <Input
                      id={`wizard-aliases-${index}`}
                      value={entry.aliases}
                      onChange={e => setEntry(index, { aliases: e.target.value })}
                      placeholder={t(interest ? 'targets.termsPlaceholder' : 'wizard.subject.aliasesPlaceholder')}
                    />
                  </div>
                </div>
              ))}
              <Button variant="outline" size="sm" onClick={() => setEntries([...entries, { name: '', aliases: '' }])}>
                <Plus className="mr-2 h-4 w-4" /> {t(interest ? 'wizard.subject.addTopic' : 'wizard.subject.addPerson')}
              </Button>
            </div>

            <div className="rounded-lg border border-gray-200 bg-gray-50 p-4">
              <p className="mb-2 text-sm font-medium text-gray-900">{t('wizard.subject.tipsTitle')}</p>
              <ul className="list-disc space-y-1.5 pl-5 text-sm text-gray-700">
                {(interest ? (['wizard.subject.tip.interest1', 'wizard.subject.tip.interest2'] as const) : (['wizard.subject.tip1', 'wizard.subject.tip2', 'wizard.subject.tip3'] as const)).map(tip => (
                  <li key={tip}>{t(tip)}</li>
                ))}
              </ul>
            </div>

            <div className="space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="wizard-investigation">{t('wizard.subject.name')}</Label>
                <Input
                  id="wizard-investigation"
                  value={name}
                  onChange={e => setName(e.target.value)}
                  placeholder={first ? t('wizard.subject.nameDefault', { name: first }) : t(interest ? 'investigation.namePlaceholder.interest' : 'wizard.subject.namePlaceholder')}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="wizard-description">{t('investigation.description')}</Label>
                <Textarea
                  id="wizard-description"
                  value={description}
                  onChange={e => setDescription(e.target.value)}
                  placeholder={t(interest ? 'wizard.subject.descriptionPlaceholder.interest' : 'investigation.descriptionPlaceholder')}
                  rows={2}
                />
                <p className="text-xs text-gray-500">{t('wizard.subject.descriptionHelp')}</p>
              </div>
            </div>

            <div className="flex items-center justify-between gap-2">
              {created || chosen ? <span /> : (
                <Button variant="ghost" onClick={() => setStep('kind')}>
                  {t('common.back')}
                </Button>
              )}
              <Button onClick={create} disabled={saving || !(name.trim() || first)}>
                {saving && <ThinkingOrb variant="working" size={16} className="mr-2" />}
                {t('wizard.subject.create')}
              </Button>
            </div>
          </div>
        );

      case 'official':
        return (
          <div className="space-y-5">
            <Heading title={t('wizard.official.title')} help={t('wizard.official.help')} />
            <dl className="space-y-2 rounded-lg border border-gray-200 bg-gray-50 p-4 text-sm">
              {(['camara', 'alesp', 'estaduais', 'tse', 'transparencia', 'pncp'] as const).map(record => (
                <div key={record} className="sm:flex sm:gap-3">
                  <dt className="font-medium text-gray-900 sm:w-44 sm:flex-shrink-0">{t(`wizard.official.${record}`)}</dt>
                  <dd className="text-gray-700">{t(`wizard.official.${record}.what`)}</dd>
                </div>
              ))}
            </dl>
            <p className="rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-900">{t('wizard.official.namesakes')}</p>
            {people.map(person => (
              <section key={person.id} className="space-y-2 rounded-lg border border-gray-200 bg-white p-4">
                <h3 className="font-semibold text-gray-900">{person.name}</h3>
                <OfficialPicker target={person} onLinked={() => created && reload(created.id).catch(() => {})} />
              </section>
            ))}
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-xs text-gray-500">{t('wizard.official.later')}</p>
              <Button onClick={next}>{t('wizard.continue')}</Button>
            </div>
          </div>
        );

      case 'sources': {
        const automatic = sources.filter(source => AUTOMATIC.some(a => a.kind === source.kind) || OFFICIAL_KINDS.includes(source.kind));
        const extra = sources.filter(source => source.origin === 'user');
        const example = EXTRA_SOURCES.find(s => s.kind === extraKind)!;
        const nameOf = (source: Source) => targets.find(target => target.id === source.targetId)?.name;
        return (
          <div className="space-y-5">
            <Heading title={t('wizard.sources.title')} help={t('wizard.sources.help')} />
            <div className="rounded-lg border border-gray-200 bg-white">
              <p className="border-b border-gray-100 px-4 py-3 text-sm font-medium text-gray-900">{t('wizard.sources.automatic')}</p>
              {automatic.length === 0 ? (
                <p className="px-4 py-3 text-sm text-gray-500">{t('wizard.sources.noAutomatic')}</p>
              ) : (
                <ul className="divide-y divide-gray-100">
                  {automatic.map(source => {
                    const Icon = AUTOMATIC.find(a => a.kind === source.kind)?.icon ?? Landmark;
                    const who = nameOf(source);
                    return (
                      <li key={source.id} className="flex items-start gap-3 px-4 py-2.5 text-sm">
                        <Icon className="mt-0.5 h-4 w-4 flex-shrink-0 text-gray-500" />
                        <span className="min-w-0">
                          <span className="block text-gray-900">
                            {t(`sources.kind.${source.kind}`)}
                            {who && <span className="text-gray-500"> · {who}</span>}
                          </span>
                          <span className="block text-xs text-gray-500">{t(`wizard.sources.what.${source.kind}` as MessageKey)}</span>
                        </span>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
            {!sources.some(source => source.kind === 'web_search') && <p className="text-sm text-gray-600">{t('wizard.sources.webSearch')}</p>}

            <div className="space-y-3 rounded-lg border border-gray-200 bg-white p-4">
              <p className="text-sm font-medium text-gray-900">{t(interest ? 'wizard.sources.extra.interest' : 'wizard.sources.extra')}</p>
              <div className="flex flex-wrap gap-2" role="group" aria-label={t('wizard.sources.extraKind')}>
                {EXTRA_SOURCES.map(option => {
                  const Icon = option.icon;
                  const selected = option.kind === extraKind;
                  return (
                    <Button key={option.kind} variant={selected ? 'default' : 'outline'} size="sm" aria-pressed={selected} onClick={() => setExtraKind(option.kind)}>
                      <Icon className="mr-1.5 h-4 w-4" /> {t(`sources.kind.${option.kind}`)}
                    </Button>
                  );
                })}
              </div>
              <p className="text-sm text-gray-700">{t(`wizard.sources.example.${extraKind}${interest ? '.interest' : ''}` as MessageKey)}</p>
              <div className="flex flex-wrap gap-2">
                <Input
                  className="min-w-[14rem] flex-1"
                  value={extraUrl}
                  onChange={e => setExtraUrl(e.target.value)}
                  onKeyDown={e => e.key === 'Enter' && addSource()}
                  placeholder={t(example.placeholder)}
                  aria-label={t(`sources.kind.${extraKind}`)}
                />
                <Button onClick={addSource} disabled={adding || !extraUrl.trim()}>
                  {adding ? <ThinkingOrb variant="working" size={16} className="mr-2" /> : <Plus className="mr-2 h-4 w-4" />}
                  {t('sources.add')}
                </Button>
              </div>
              {extra.length > 0 && (
                <ul className="space-y-1 text-sm">
                  {extra.map(source => (
                    <li key={source.id} className="flex items-center gap-2 text-gray-700">
                      <Check className="h-4 w-4 flex-shrink-0 text-green-700" />
                      <span className="truncate" title={source.url}>
                        {t(`sources.kind.${source.kind}`)}: {source.title ?? source.url}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-xs text-gray-500">{t('wizard.sources.later')}</p>
              <Button onClick={next}>{t(extra.length ? 'wizard.continue' : 'wizard.sources.skip')}</Button>
            </div>
          </div>
        );
      }

      case 'done': {
        const linked = people.some(person => person.official.length > 0);
        // The questions are about the first person (or theme) followed
        const subject = (interest ? targets[0] : people[0])?.name ?? '';
        const expect: MessageKey[] = interest
          ? ['wizard.done.expect.minutes', 'wizard.done.expect.interestFeed']
          : ['wizard.done.expect.minutes', ...(linked ? (['wizard.done.expect.official'] as const) : []), 'wizard.done.expect.analysis', 'wizard.done.expect.review'];
        const questions: MessageKey[] = interest
          ? ['wizard.done.ask.interest1', 'wizard.done.ask.interest2']
          : ['wizard.done.ask.politics1', 'wizard.done.ask.politics2', 'wizard.done.ask.politics3'];
        return (
          <div className="space-y-5">
            <Heading title={t('wizard.done.title')} help={t('wizard.done.help')} />
            <div className="rounded-lg border border-gray-200 bg-white p-4">
              <p className="mb-2 text-sm font-medium text-gray-900">{t('wizard.done.expectTitle')}</p>
              <ul className="space-y-2 text-sm text-gray-700">
                {expect.map(line => (
                  <li key={line} className="flex gap-2">
                    <Check className="mt-0.5 h-4 w-4 flex-shrink-0 text-green-700" />
                    <span>{t(line)}</span>
                  </li>
                ))}
              </ul>
            </div>
            {subject && (
              <div className="rounded-lg border border-gray-200 bg-gray-50 p-4">
                <p className="mb-2 text-sm font-medium text-gray-900">{t('wizard.done.askTitle')}</p>
                <ul className="space-y-1.5 text-sm text-gray-700">
                  {questions.map(question => (
                    <li key={question}>“{t(question, { name: subject })}”</li>
                  ))}
                </ul>
              </div>
            )}
            {!interest && <p className="text-sm text-gray-600">{t('wizard.done.live')}</p>}
            <div className="flex flex-wrap justify-end gap-2">
              <Button variant="outline" onClick={() => finish('/sources')}>
                <Radio className="mr-2 h-4 w-4" /> {t('wizard.done.sources')}
              </Button>
              <Button onClick={() => finish('/investigation')}>{t('wizard.done.home')}</Button>
            </div>
          </div>
        );
      }
    }
  })();

  return (
    <div ref={top} className="scroll-mt-6 space-y-6">
      <Steps steps={steps} current={step} />
      {body}
    </div>
  );
}
