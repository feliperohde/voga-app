'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { ArrowRight, Cloud, Cpu, Download } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';
import { useI18n } from '@/i18n';
import { ProviderConnect } from '@/components/ProviderConnect';
import { BuiltinModel, builtinService, ComputerProfile, DownloadProgress, formatSize, isAvailable } from '@/services/builtinService';
import { AiSettings, settingsService } from '@/services/settingsService';

type Setup =
  | { state: 'loading' }
  | { state: 'ready' }
  /** A cloud or Ollama provider with no model chosen. */
  | { state: 'choose' }
  /** Built-in models still to download. */
  | { state: 'download'; missing: BuiltinModel[]; settings: AiSettings };

/** What is missing for Voga to read and answer. */
async function check(): Promise<Setup> {
  const { keys: _keys, ...settings } = await settingsService.get();
  const missing: BuiltinModel[] = [];
  if (settings.embeddingProvider === 'builtin') {
    const models = await builtinService.list('embedding');
    const wanted = models.find(m => m.name === settings.embeddingModel);
    if (wanted && !isAvailable(wanted)) missing.push(wanted);
  }
  if (settings.provider === 'builtin-ai') {
    const models = await builtinService.list('summary');
    // No model chosen: any one downloaded does, else the one for this computer
    const name = settings.model || (models.some(isAvailable) ? '' : await builtinService.recommended());
    const wanted = models.find(m => m.name === name);
    if (wanted && !isAvailable(wanted)) missing.push(wanted);
  } else if (!settings.model) {
    return { state: 'choose' };
  }
  return missing.length ? { state: 'download', missing, settings } : { state: 'ready' };
}

/** Home: until the AI can run, what to download (built-in) or to choose (other providers). */
export function AiSetup() {
  const { t } = useI18n();
  const router = useRouter();
  const [setup, setSetup] = useState<Setup>({ state: 'loading' });
  const [current, setCurrent] = useState<DownloadProgress | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [profile, setProfile] = useState<ComputerProfile | null>(null);
  /** On a modest computer, the person chose the built-in models anyway. */
  const [local, setLocal] = useState(false);

  const refresh = useCallback(() => {
    check()
      .then(setSetup)
      .catch(() => setSetup({ state: 'choose' }));
  }, []);

  useEffect(() => {
    builtinService
      .computerProfile()
      .then(setProfile)
      .catch(() => {});
    refresh();
    const unlisten = builtinService.onProgress(update => {
      if (update.status === 'downloading') setCurrent(update);
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [refresh]);

  if (setup.state === 'loading' || setup.state === 'ready') return null;

  if (setup.state === 'choose') {
    return (
      <div className="flex items-center justify-between gap-4 rounded-lg border border-amber-200 bg-amber-50 p-4">
        <p className="text-sm text-amber-900">{t('home.setupAi')}</p>
        <Button variant="outline" size="sm" onClick={() => router.push('/settings')}>
          {t('home.openSettings')} <ArrowRight className="ml-2 h-4 w-4" />
        </Button>
      </div>
    );
  }

  const { missing, settings } = setup;
  const total = missing.reduce((sum, m) => sum + m.size_mb, 0);

  // A modest computer: the AI the person already uses, before gigabytes of slow models
  if (profile?.basic && settings.provider === 'builtin-ai' && !local && !downloading) {
    return (
      <div className="space-y-4 rounded-lg border border-blue-200 bg-blue-50 p-4">
        <div className="flex items-start gap-3">
          <Cloud className="mt-0.5 h-5 w-5 flex-shrink-0 text-blue-700" />
          <div className="space-y-1">
            <p className="text-sm font-medium text-blue-950">{t('home.basic.title')}</p>
            <p className="text-sm text-blue-900">
              {t('home.basic.help', { ram: profile.ramGb, gpu: t(profile.gpu ? 'home.basic.gpu' : 'home.basic.noGpu') })}
            </p>
          </div>
        </div>
        <div className="rounded-md border border-blue-100 bg-white p-4">
          <ProviderConnect onConnected={refresh} />
        </div>
        <div className="flex justify-end">
          <Button variant="ghost" size="sm" onClick={() => setLocal(true)}>
            {t('home.basic.local', { size: formatSize(total) })}
          </Button>
        </div>
      </div>
    );
  }
  const downloadAll = async () => {
    setDownloading(true);
    try {
      // The index model first: it is smaller, and collection can index while the other arrives
      for (const model of missing) {
        setCurrent({ model: model.name, progress: 0, status: 'downloading' });
        await builtinService.download(model.name);
      }
      const chat = missing.find(m => m.kind === 'summary');
      if (chat && settings.provider === 'builtin-ai' && !settings.model) await settingsService.save({ ...settings, model: chat.name });
      toast.success(t('home.builtin.ready'));
    } catch (error) {
      if (!String(error).startsWith('CANCELLED')) toast.error(t('builtin.downloadFailed'), { description: String(error) });
    } finally {
      setDownloading(false);
      setCurrent(null);
      refresh();
    }
  };
  const currentModel = current && missing.find(m => m.name === current.model);

  return (
    <div className="space-y-3 rounded-lg border border-blue-200 bg-blue-50 p-4">
      <div className="flex items-start gap-3">
        <Cpu className="mt-0.5 h-5 w-5 flex-shrink-0 text-blue-700" />
        <div className="space-y-1">
          <p className="text-sm font-medium text-blue-950">{t('home.builtin.title')}</p>
          <p className="text-sm text-blue-900">{t('home.builtin.help', { size: formatSize(total) })}</p>
          <ul className="text-xs text-blue-800">
            {missing.map(m => (
              <li key={m.name}>
                {m.display_name} · {formatSize(m.size_mb)}
              </li>
            ))}
          </ul>
        </div>
      </div>
      {downloading && current ? (
        <div className="space-y-1">
          <div className="flex justify-between text-xs text-blue-900">
            <span>{t('home.builtin.downloading', { name: currentModel?.display_name ?? current.model })}</span>
            <span>
              {current.progress}%{current.speed_mbps ? ` · ${t('builtin.speed', { speed: current.speed_mbps.toFixed(1) })}` : ''}
            </span>
          </div>
          <Progress value={current.progress} />
        </div>
      ) : (
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => router.push('/settings')}>
            {t('home.builtin.other')}
          </Button>
          <Button size="sm" onClick={downloadAll}>
            <Download className="mr-2 h-4 w-4" /> {t('home.builtin.download')}
          </Button>
        </div>
      )}
    </div>
  );
}
