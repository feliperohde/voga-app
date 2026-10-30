'use client';

import { useEffect, useState } from 'react';
import { CheckCircle2, ExternalLink } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { openExternal } from '@/lib/openExternal';
import type { CollectSettings as Settings, NewsImages, WebSearchSettings } from '@/services/settingsService';
import { useI18n } from '@/i18n';
import { settingsService } from '@/services/settingsService';


/** The optional web search service: Brave Search (a key) or a SearXNG (an address). */
function WebSearch() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<WebSearchSettings | null>(null);
  const [testing, setTesting] = useState(false);

  useEffect(() => {
    settingsService
      .webSearch()
      .then(setSettings)
      .catch(error => console.error('Failed to load web search settings:', error));
  }, []);
  if (!settings) return null;
  const set = (patch: Partial<WebSearchSettings>) => setSettings({ ...settings, ...patch });

  const saveAndTest = async () => {
    setTesting(true);
    try {
      await settingsService.saveWebSearch(settings);
      setSettings({ ...settings, key: '', hasKey: settings.hasKey || !!settings.key });
      if (settings.provider) {
        const count = await settingsService.testWebSearch('Câmara dos Deputados');
        toast.success(t('settings.collect.webSearchOk', { count }));
      } else {
        toast.success(t('common.saved'));
      }
    } catch (error) {
      toast.error(t('settings.collect.webSearchFailed'), { description: String(error) });
    } finally {
      setTesting(false);
    }
  };

  return (
    <section className="space-y-3 rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
      <div>
        <h3 className="text-lg font-semibold text-gray-900">{t('settings.collect.webSearch')}</h3>
        <p className="text-sm text-gray-600">{t('settings.collect.webSearchHelp')}</p>
      </div>
      <SelectField
        value={settings.provider}
        onValueChange={provider => set({ provider: provider as WebSearchSettings['provider'] })}
        options={[
          { value: '', label: t('settings.collect.webSearchOff') },
          { value: 'brave', label: 'Brave Search' },
          { value: 'searxng', label: 'SearXNG' },
        ]}
      />
      {settings.provider === 'brave' && (
        <div className="space-y-1.5">
          <Label htmlFor="brave-key">{t('settings.ai.apiKey')}</Label>
          <Input
            id="brave-key"
            type="password"
            value={settings.key}
            onChange={e => set({ key: e.target.value })}
            placeholder={settings.hasKey ? t('settings.ai.keySaved') : t('settings.ai.keyNeeded')}
            autoComplete="off"
          />
          <button type="button" onClick={() => openExternal('https://brave.com/search/api/')} className="flex items-center gap-1 text-xs text-blue-700 hover:underline">
            {t('settings.collect.braveKey')} <ExternalLink className="h-3 w-3" />
          </button>
        </div>
      )}
      {settings.provider === 'searxng' && (
        <div className="space-y-1.5">
          <Label htmlFor="searxng">{t('settings.collect.searxngEndpoint')}</Label>
          <Input id="searxng" value={settings.endpoint} onChange={e => set({ endpoint: e.target.value })} placeholder="http://localhost:8080" />
          <p className="text-xs text-gray-500">{t('settings.collect.searxngHelp')}</p>
        </div>
      )}
      <div className="flex justify-end">
        <Button onClick={saveAndTest} disabled={testing || (settings.provider === 'brave' && !settings.key && !settings.hasKey) || (settings.provider === 'searxng' && !settings.endpoint.trim())}>
          {testing ? <ThinkingOrb variant="working" size={16} className="mr-2" /> : <CheckCircle2 className="mr-2 h-4 w-4" />}
          {settings.provider ? t('settings.flowsint.saveAndTest') : t('common.save')}
        </Button>
      </div>
    </section>
  );
}

/** Settings › Collection. */
export function CollectSettings() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<Settings | null>(null);

  useEffect(() => {
    settingsService
      .collect()
      .then(setSettings)
      .catch(error => console.error('Failed to load collection settings:', error));
  }, []);

  const save = (patch: Partial<Settings>) => {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next);
    settingsService
      .saveCollect(next)
      .then(() => toast.success(t('common.saved')))
      .catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));
  };

  if (settings === null) return null;
  return (
    <div className="space-y-6">
      <section className="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h3 className="text-lg font-semibold text-gray-900">{t('settings.collect.analysis')}</h3>
            <p className="text-sm text-gray-600">{t('settings.collect.analysisHelp')}</p>
          </div>
          <Switch checked={settings.analysis} onCheckedChange={analysis => save({ analysis })} />
        </div>
      </section>
      <section className="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h3 className="text-lg font-semibold text-gray-900">{t('settings.collect.fullProposals')}</h3>
            <p className="text-sm text-gray-600">{t('settings.collect.fullProposalsHelp')}</p>
          </div>
          <Switch checked={settings.fullProposals} onCheckedChange={fullProposals => save({ fullProposals })} />
        </div>
      </section>
      <section className="space-y-3 rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <div>
          <h3 className="text-lg font-semibold text-gray-900">{t('settings.collect.newsImages')}</h3>
          <p className="text-sm text-gray-600">{t('settings.collect.newsImagesHelp')}</p>
        </div>
        <SelectField
          value={settings.newsImages}
          onValueChange={value => save({ newsImages: value as NewsImages })}
          aria-label={t('settings.collect.newsImages')}
          options={(['both', 'image', 'url'] as const).map(mode => ({ value: mode, label: t(`settings.collect.newsImages.${mode}`) }))}
        />
        <p className="text-xs text-gray-500">{t(`settings.collect.newsImages.${settings.newsImages}Help`)}</p>
      </section>
      <WebSearch />
      <section className="space-y-2 rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900">{t('settings.collect.politeness')}</h3>
        <p className="text-sm text-gray-600">{t('settings.collect.politenessHelp')}</p>
      </section>
    </div>
  );
}
