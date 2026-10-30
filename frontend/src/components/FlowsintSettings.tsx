'use client';

import { useCallback, useEffect, useState } from 'react';
import { CheckCircle2, Copy, Download, ExternalLink, Eye, EyeOff, Play, Square, Trash2, UserPlus } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { useConfirm } from '@/components/ui/confirm-dialog';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { MessageKey, useI18n } from '@/i18n';
import { openExternal } from '@/lib/openExternal';
import { FlowsintProgress, FlowsintSettings as Settings, flowsintService, LocalFlowsint as Status } from '@/services/graphService';

const DOCKER_DESKTOP = 'https://www.docker.com/products/docker-desktop/';
const ORBSTACK = 'https://orbstack.dev';

function Notice({ tone, children }: { tone: 'amber' | 'green' | 'gray'; children: React.ReactNode }) {
  const tones = {
    amber: 'border-amber-200 bg-amber-50 text-amber-900',
    green: 'border-green-200 bg-green-50 text-green-900',
    gray: 'border-gray-200 bg-gray-50 text-gray-800',
  };
  return <div className={`space-y-3 rounded-lg border p-4 text-sm ${tones[tone]}`}>{children}</div>;
}

/** A value to copy: the login of the flowsint web app. */
function Secret({ label, value, hidden }: { label: string; value: string; hidden?: boolean }) {
  const { t } = useI18n();
  const [shown, setShown] = useState(!hidden);
  return (
    <div className="flex items-center gap-2">
      <span className="w-20 flex-shrink-0 text-xs text-gray-500">{label}</span>
      <code className="min-w-0 flex-1 truncate rounded bg-white/70 px-2 py-1 text-xs">{shown ? value : '••••••••••••'}</code>
      {hidden && (
        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setShown(!shown)} aria-label={t('settings.flowsint.local.show')}>
          {shown ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
        </Button>
      )}
      <Button
        variant="ghost"
        size="icon"
        className="h-7 w-7"
        onClick={() => navigator.clipboard.writeText(value).then(() => toast.success(t('settings.flowsint.local.copied')))}
        aria-label={t('settings.flowsint.local.copy')}
      >
        <Copy className="h-3.5 w-3.5" />
      </Button>
    </div>
  );
}

/** flowsint on this computer: Voga installs, starts and stops it through Docker. */
function LocalFlowsint() {
  const { t } = useI18n();
  const confirm = useConfirm();
  const [status, setStatus] = useState<Status | null>(null);
  const [progress, setProgress] = useState<FlowsintProgress | null>(null);
  const [acting, setActing] = useState(false);

  const refresh = useCallback(
    () =>
      flowsintService
        .localStatus()
        .then(next => {
          setStatus(next);
          if (!next.busy) setProgress(null);
          else if (next.progress) setProgress(next.progress);
        })
        .catch(error => console.error('Failed to read the local flowsint:', error)),
    [],
  );

  useEffect(() => {
    refresh();
    // Docker and the containers change on their own (Docker opened, flowsint starting)
    const timer = setInterval(refresh, 4000);
    const unlisten = flowsintService.onProgress(setProgress);
    return () => {
      clearInterval(timer);
      unlisten.then(fn => fn());
    };
  }, [refresh]);

  if (!status) return <ThinkingOrb variant="working" size={20} />;

  const act = async (action: () => Promise<void>, done?: string) => {
    setActing(true);
    try {
      await action();
      if (done) toast.success(done);
    } catch (error) {
      toast.error(t('settings.flowsint.local.failed'), { description: String(error) });
    } finally {
      setActing(false);
      setProgress(null);
      refresh();
    }
  };

  const remove = async () => {
    const ok = await confirm({
      title: t('settings.flowsint.local.removeTitle'),
      description: t('settings.flowsint.local.removeHelp'),
      destructive: true,
      confirmLabel: t('settings.flowsint.local.remove'),
    });
    if (ok) act(flowsintService.remove, t('settings.flowsint.local.removed'));
  };

  if (status.engine === 'missing') {
    return (
      <Notice tone="amber">
        <p>{t('settings.flowsint.local.noDocker')}</p>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={() => openExternal(DOCKER_DESKTOP)}>
            Docker Desktop <ExternalLink className="ml-2 h-3.5 w-3.5" />
          </Button>
          <Button variant="outline" size="sm" onClick={() => openExternal(ORBSTACK)}>
            OrbStack (macOS) <ExternalLink className="ml-2 h-3.5 w-3.5" />
          </Button>
        </div>
      </Notice>
    );
  }

  if (status.engine === 'stopped') {
    return (
      <Notice tone="amber">
        <p>{t('settings.flowsint.local.dockerStopped')}</p>
        {status.engineApp && (
          <Button variant="outline" size="sm" onClick={() => act(flowsintService.openDocker)}>
            {t('settings.flowsint.local.openDocker', { app: status.engineApp })}
          </Button>
        )}
      </Notice>
    );
  }

  if (acting || status.busy) {
    const stage = progress?.stage ?? 'start';
    return (
      <Notice tone="gray">
        <div className="flex items-center gap-3">
          <ThinkingOrb variant="background" size={20} />
          <div className="min-w-0">
            <p className="font-medium">{t(`settings.flowsint.local.stage.${stage}` as MessageKey)}</p>
            {progress?.message && <p className="truncate font-mono text-xs text-gray-500">{progress.message}</p>}
          </div>
        </div>
      </Notice>
    );
  }

  if (!status.installed) {
    return (
      <Notice tone="gray">
        <p>{t('settings.flowsint.local.installHelp', { version: status.version })}</p>
        <div className="flex justify-end">
          <Button size="sm" onClick={() => act(flowsintService.install, t('settings.flowsint.local.installed'))}>
            <Download className="mr-2 h-4 w-4" /> {t('settings.flowsint.local.install')}
          </Button>
        </div>
      </Notice>
    );
  }

  const running = status.stack === 'running';
  return (
    <div className="space-y-3">
      <Notice tone={running ? 'green' : 'gray'}>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="font-medium">
            {running
              ? t('settings.flowsint.local.running', { url: status.url ?? '' })
              : status.stack === 'starting'
                ? t('settings.flowsint.local.starting')
                : t('settings.flowsint.local.stopped')}
          </p>
          <div className="flex gap-2">
            {running ? (
              <>
                <Button variant="outline" size="sm" onClick={() => status.url && openExternal(status.url)}>
                  <ExternalLink className="mr-2 h-4 w-4" /> {t('settings.flowsint.local.open')}
                </Button>
                <Button variant="outline" size="sm" onClick={() => act(flowsintService.stop)}>
                  <Square className="mr-2 h-4 w-4" /> {t('settings.flowsint.local.stop')}
                </Button>
              </>
            ) : (
              <Button size="sm" onClick={() => act(flowsintService.start)} disabled={status.stack === 'starting'}>
                <Play className="mr-2 h-4 w-4" /> {t('settings.flowsint.local.start')}
              </Button>
            )}
          </div>
        </div>
        {status.password && (
          <div className="space-y-1">
            <p className="text-xs text-gray-600">{t('settings.flowsint.local.login')}</p>
            <Secret label={t('settings.flowsint.local.email')} value={status.email} />
            <Secret label={t('settings.flowsint.password')} value={status.password} hidden />
          </div>
        )}
      </Notice>
      <div className="flex justify-end">
        <Button variant="ghost" size="sm" className="text-red-600 hover:text-red-700" onClick={remove}>
          <Trash2 className="mr-2 h-4 w-4" /> {t('settings.flowsint.local.remove')}
        </Button>
      </div>
    </div>
  );
}

/** A flowsint server the person runs elsewhere, and the account Voga uses there. */
function ServerFlowsint({ settings, setSettings }: { settings: Settings; setSettings: (s: Settings) => void }) {
  const { t } = useI18n();
  const [busy, setBusy] = useState<'test' | 'create' | null>(null);
  const set = (patch: Partial<Settings>) => setSettings({ ...settings, ...patch });

  const run = async (action: 'test' | 'create') => {
    setBusy(action);
    try {
      await flowsintService.save(settings);
      if (action === 'create') {
        await flowsintService.createAccount();
        toast.success(t('settings.flowsint.created'));
      }
      await flowsintService.test();
      toast.success(t('settings.flowsint.ok'));
      setSettings({ ...settings, password: '', hasPassword: settings.hasPassword || !!settings.password });
    } catch (error) {
      toast.error(t('settings.flowsint.failed'), { description: String(error) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <Label htmlFor="fs-endpoint">{t('settings.flowsint.endpoint')}</Label>
        <Input id="fs-endpoint" value={settings.endpoint} onChange={e => set({ endpoint: e.target.value })} placeholder="https://flowsint.example.org" />
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="fs-email">{t('settings.flowsint.email')}</Label>
          <Input id="fs-email" value={settings.email} onChange={e => set({ email: e.target.value })} autoComplete="off" />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="fs-password">{t('settings.flowsint.password')}</Label>
          <Input
            id="fs-password"
            type="password"
            value={settings.password}
            onChange={e => set({ password: e.target.value })}
            placeholder={settings.hasPassword ? t('settings.ai.keySaved') : ''}
            autoComplete="new-password"
          />
        </div>
      </div>
      <div className="flex justify-end gap-2">
        <Button variant="outline" onClick={() => run('create')} disabled={!!busy || !settings.endpoint || !settings.email || !settings.password}>
          {busy === 'create' ? <ThinkingOrb variant="working" size={16} className="mr-2" /> : <UserPlus className="mr-2 h-4 w-4" />}
          {t('settings.flowsint.createAccount')}
        </Button>
        <Button onClick={() => run('test')} disabled={!!busy || !settings.endpoint || !settings.email || (!settings.password && !settings.hasPassword)}>
          {busy === 'test' ? <ThinkingOrb variant="working" size={16} className="mr-2" /> : <CheckCircle2 className="mr-2 h-4 w-4" />}
          {t('settings.flowsint.saveAndTest')}
        </Button>
      </div>
    </div>
  );
}

/** Settings › flowsint: where investigations are sent. On this computer by default; a server is optional. */
export function FlowsintSettings() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<Settings | null>(null);

  useEffect(() => {
    flowsintService
      .settings()
      .then(setSettings)
      .catch(error => console.error('Failed to load flowsint settings:', error));
  }, []);

  if (!settings) return null;

  const setMode = (mode: Settings['mode']) => {
    const next = { ...settings, mode };
    setSettings(next);
    flowsintService.save(next).catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));
  };

  return (
    <section className="space-y-4 rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
      <div>
        <h3 className="text-lg font-semibold text-gray-900">flowsint</h3>
        <p className="text-sm text-gray-600">{t('settings.flowsint.help')}</p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="fs-mode">{t('settings.flowsint.mode')}</Label>
        <SelectField
          id="fs-mode"
          value={settings.mode}
          onValueChange={value => setMode(value as Settings['mode'])}
          options={[
            { value: 'local', label: t('settings.flowsint.modeLocal') },
            { value: 'server', label: t('settings.flowsint.modeServer') },
          ]}
        />
      </div>
      {settings.mode === 'local' ? <LocalFlowsint /> : <ServerFlowsint settings={settings} setSettings={setSettings} />}
      <p className="text-xs text-gray-500">{t('settings.flowsint.what')}</p>
    </section>
  );
}
