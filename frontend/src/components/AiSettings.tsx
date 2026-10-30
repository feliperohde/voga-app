'use client';

import { useCallback, useEffect, useState } from 'react';
import { CheckCircle2, RefreshCw } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useI18n } from '@/i18n';
import { BuiltinModels } from '@/components/BuiltinModels';
import { ProviderConnect } from '@/components/ProviderConnect';
import { LiveSettings } from '@/components/live/LiveSettings';
import { builtinService, isAvailable } from '@/services/builtinService';
import { AiProvider, AiSettings as Settings, EmbeddingProvider, settingsService } from '@/services/settingsService';

const PROVIDERS: { value: AiProvider; label: string }[] = [
  // The label of 'builtin-ai' is translated
  { value: 'builtin-ai', label: '' },
  { value: 'ollama', label: 'Ollama' },
  { value: 'claude', label: 'Claude (Anthropic)' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'openrouter', label: 'OpenRouter' },
  { value: 'gemini', label: 'Google Gemini' },
  { value: 'groq', label: 'Groq' },
  { value: 'deepseek', label: 'DeepSeek' },
  { value: 'custom-openai', label: 'OpenAI-compatible' },
  { value: 'claude-code', label: 'Claude Code' },
];

/** Providers with no key, or where it is optional. */
const KEYLESS: AiProvider[] = ['builtin-ai', 'ollama', 'claude-code'];

const BUILTIN_EMBEDDING = 'bge-m3-q8';
const OLLAMA_EMBEDDING = 'bge-m3';

/** The built-in chat model to show as chosen: the best one downloaded, else the one for this computer. */
async function defaultBuiltinModel(): Promise<string> {
  const models = await builtinService.list('summary');
  return models.find(isAvailable)?.name ?? (await builtinService.recommended());
}

function Card({ title, help, children }: { title: string; help: string; children: React.ReactNode }) {
  return (
    <section className="space-y-4 rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
      <div>
        <h3 className="text-lg font-semibold text-gray-900">{title}</h3>
        <p className="text-sm text-gray-600">{help}</p>
      </div>
      {children}
    </section>
  );
}

function Field({ id, label, children }: { id: string; label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      {children}
    </div>
  );
}

/** Settings › AI: the chat model every feature uses and the model of the search index; built-in by default,
 * Ollama (this computer or the network) and cloud providers optional. */
export function AiSettings() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [keys, setKeys] = useState<AiProvider[]>([]);
  const [apiKey, setApiKey] = useState('');
  const [ollamaModels, setOllamaModels] = useState<string[] | null>(null);
  const [ollamaError, setOllamaError] = useState<string | null>(null);
  const [busy, setBusy] = useState<'save' | 'test' | null>(null);
  /** Bumped when a provider is connected: the live settings read theirs again. */
  const [connected, setConnected] = useState(0);

  const reload = useCallback(
    () =>
      settingsService
        .get()
        .then(async ({ keys, ...saved }) => {
          // No built-in model chosen: the core uses the best one downloaded, show that one
          if (saved.provider === 'builtin-ai' && !saved.model) saved = { ...saved, model: await defaultBuiltinModel().catch(() => '') };
          setSettings(saved);
          setKeys(keys);
        })
        .catch(error => toast.error(t('settings.ai.loadFailed'), { description: String(error) })),
    [t],
  );
  useEffect(() => {
    reload();
  }, [reload]);

  const loadOllamaModels = useCallback((endpoint: string | null) => {
    setOllamaError(null);
    settingsService
      .ollamaModels(endpoint ?? undefined)
      .then(setOllamaModels)
      .catch(error => {
        setOllamaModels(null);
        setOllamaError(String(error));
      });
  }, []);

  const usesOllama = settings?.provider === 'ollama' || settings?.embeddingProvider === 'ollama';
  useEffect(() => {
    if (settings && usesOllama) loadOllamaModels(settings.ollamaEndpoint);
    // When the settings arrive or Ollama starts being used: the endpoint field reloads on blur
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings === null, usesOllama]);

  if (!settings) return <ThinkingOrb variant="working" size={20} />;

  const set = (patch: Partial<Settings>) => setSettings({ ...settings, ...patch });
  const setProvider = (provider: AiProvider) => {
    set({ provider, model: '' });
    if (provider === 'builtin-ai') defaultBuiltinModel().then(model => setSettings(s => (s && s.provider === 'builtin-ai' && !s.model ? { ...s, model } : s)));
  };
  const setEmbeddingProvider = (embeddingProvider: EmbeddingProvider) =>
    set({ embeddingProvider, embeddingModel: embeddingProvider === 'builtin' ? BUILTIN_EMBEDDING : embeddingProvider === 'ollama' ? OLLAMA_EMBEDDING : '' });
  const needsKey = !KEYLESS.includes(settings.provider) && settings.provider !== 'custom-openai';
  const hasKey = keys.includes(settings.provider);

  const save = async () => {
    setBusy('save');
    try {
      await settingsService.save(settings);
      if (apiKey.trim()) {
        await settingsService.saveApiKey(settings.provider, apiKey.trim());
        setKeys([...keys.filter(k => k !== settings.provider), settings.provider]);
        setApiKey('');
      }
      toast.success(t('common.saved'));
    } catch (error) {
      toast.error(t('common.saveFailed'), { description: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const test = async () => {
    setBusy('test');
    try {
      await save();
      const answer = await settingsService.test();
      toast.success(t('settings.ai.testOk'), { description: answer.slice(0, 120) });
    } catch (error) {
      toast.error(t('settings.ai.testFailed'), { description: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const modelOptions = (selected: string) =>
    [...new Set([...(ollamaModels ?? []), selected].filter(Boolean))].map(name => ({ value: name, label: name }));

  return (
    <div className="space-y-6">
      <Card title={t('connect.title')} help={t('connect.help')}>
        <ProviderConnect
          onConnected={() => {
            reload();
            setConnected(n => n + 1);
          }}
        />
      </Card>

      <Card title={t('settings.ai.model')} help={t('settings.ai.modelHelp')}>
        <Field id="provider" label={t('settings.ai.provider')}>
          <SelectField
            value={settings.provider}
            onValueChange={value => setProvider(value as AiProvider)}
            options={PROVIDERS.map(p => (p.value === 'builtin-ai' ? { ...p, label: t('settings.ai.builtin') } : p))}
          />
        </Field>

        {settings.provider === 'builtin-ai' && (
          <>
            <p className="text-xs text-gray-500">{t('settings.ai.builtinHelp')}</p>
            <BuiltinModels kind="summary" selected={settings.model} onSelect={model => set({ model })} />
          </>
        )}

        {settings.provider === 'custom-openai' && (
          <Field id="custom-endpoint" label={t('settings.ai.customEndpoint')}>
            <Input
              id="custom-endpoint"
              value={settings.customEndpoint ?? ''}
              onChange={e => set({ customEndpoint: e.target.value })}
              placeholder="http://localhost:8000/v1"
            />
          </Field>
        )}

        {settings.provider !== 'builtin-ai' && (
          <Field id="model" label={t('settings.ai.modelName')}>
            {settings.provider === 'ollama' && ollamaModels ? (
              <SelectField value={settings.model} onValueChange={model => set({ model })} options={modelOptions(settings.model)} placeholder={t('settings.ai.pickModel')} />
            ) : (
              <Input
                id="model"
                value={settings.model}
                onChange={e => set({ model: e.target.value })}
                placeholder={settings.provider === 'claude' ? 'claude-sonnet-5-5' : settings.provider === 'claude-code' ? 'sonnet' : ''}
              />
            )}
          </Field>
        )}

        {!KEYLESS.includes(settings.provider) && (
          <Field id="api-key" label={t('settings.ai.apiKey')}>
            <Input
              id="api-key"
              type="password"
              value={apiKey}
              onChange={e => setApiKey(e.target.value)}
              placeholder={hasKey ? t('settings.ai.keySaved') : needsKey ? t('settings.ai.keyNeeded') : t('settings.ai.keyOptional')}
            />
          </Field>
        )}
        {settings.provider === 'claude-code' && <p className="text-xs text-gray-500">{t('settings.ai.claudeCodeHelp')}</p>}
      </Card>

      <Card title={t('settings.ai.index')} help={t('settings.ai.indexHelp')}>
        <Field id="embedding-provider" label={t('settings.ai.indexProvider')}>
          <SelectField
            value={settings.embeddingProvider}
            onValueChange={value => setEmbeddingProvider(value as EmbeddingProvider)}
            options={[
              { value: 'builtin', label: t('settings.ai.indexBuiltin') },
              { value: 'ollama', label: 'Ollama' },
              { value: 'openai', label: 'OpenAI' },
              { value: 'gemini', label: 'Google Gemini' },
              { value: 'custom-openai', label: t('connect.custom') },
            ]}
          />
        </Field>
        {settings.embeddingProvider === 'builtin' && (
          <BuiltinModels kind="embedding" selected={settings.embeddingModel} onSelect={embeddingModel => set({ embeddingModel })} />
        )}
        {['openai', 'gemini', 'custom-openai'].includes(settings.embeddingProvider) && (
          <Field id="embedding-external" label={t('settings.ai.embeddingModel')}>
            <Input id="embedding-external" value={settings.embeddingModel} onChange={e => set({ embeddingModel: e.target.value })} placeholder="text-embedding-3-small" />
            <p className="text-xs text-gray-500">{t('settings.ai.externalIndexHelp')}</p>
          </Field>
        )}
        <p className="text-xs text-gray-500">{t('settings.ai.reindexHelp')}</p>
      </Card>

      {usesOllama && (
        <Card title={t('settings.ai.ollama')} help={t('settings.ai.ollamaHelp')}>
          <Field id="ollama-endpoint" label={t('settings.ai.ollamaEndpoint')}>
            <div className="flex gap-2">
              <Input
                id="ollama-endpoint"
                value={settings.ollamaEndpoint ?? ''}
                onChange={e => set({ ollamaEndpoint: e.target.value })}
                onBlur={() => loadOllamaModels(settings.ollamaEndpoint)}
                placeholder="http://localhost:11434"
              />
              <Button variant="outline" size="icon" onClick={() => loadOllamaModels(settings.ollamaEndpoint)} aria-label={t('settings.ai.refresh')}>
                <RefreshCw className="h-4 w-4" />
              </Button>
            </div>
            {ollamaError ? (
              <p className="text-xs text-red-600">{ollamaError}</p>
            ) : (
              ollamaModels && <p className="text-xs text-gray-500">{t('settings.ai.ollamaModels', { count: ollamaModels.length })}</p>
            )}
          </Field>
          {settings.embeddingProvider === 'ollama' && (
            <Field id="embedding-model" label={t('settings.ai.embeddingModel')}>
              {ollamaModels ? (
                <SelectField value={settings.embeddingModel} onValueChange={embeddingModel => set({ embeddingModel })} options={modelOptions(settings.embeddingModel)} />
              ) : (
                <Input id="embedding-model" value={settings.embeddingModel} onChange={e => set({ embeddingModel: e.target.value })} />
              )}
              <p className="text-xs text-gray-500">{t('settings.ai.embeddingHelp')}</p>
            </Field>
          )}
        </Card>
      )}

      <div className="flex justify-end gap-2">
        <Button variant="outline" onClick={test} disabled={!!busy || !settings.model}>
          {busy === 'test' ? <ThinkingOrb variant="thinking" size={16} className="mr-2" /> : <CheckCircle2 className="mr-2 h-4 w-4" />}
          {t('settings.ai.test')}
        </Button>
        <Button onClick={save} disabled={!!busy}>
          {busy === 'save' && <ThinkingOrb variant="working" size={16} className="mr-2" />}
          {t('common.save')}
        </Button>
      </div>

      <Card title={t('live.settings.title')} help={t('live.settings.help')}>
        <LiveSettings key={connected} />
      </Card>
    </div>
  );
}
