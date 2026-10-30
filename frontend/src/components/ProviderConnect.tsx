'use client';

import { useState } from 'react';
import { ExternalLink, Plug } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useI18n } from '@/i18n';
import { openExternal } from '@/lib/openExternal';
import { CONNECTABLE, modelsFor, pickModels } from '@/lib/providers';
import { liveService } from '@/services/liveService';
import { AiProvider, EmbeddingProvider, settingsService } from '@/services/settingsService';

const BUILTIN_EMBEDDING = 'bge-m3-q8';
/** "Neste computador", in the selects of what a provider does. */
const HERE = '';

/** One AI for everything: the provider and its key once, and Voga uses it for answers and
 * analysis, and for search and live transcription where it offers them (the built-in models for
 * the rest). `onConnected` hears it saved. */
export function ProviderConnect({ onConnected }: { onConnected?: () => void }) {
  const { t } = useI18n();
  const [provider, setProvider] = useState<AiProvider>('openai');
  const [key, setKey] = useState('');
  const [endpoint, setEndpoint] = useState('');
  const [ids, setIds] = useState<string[] | null>(null);
  const [chosen, setChosen] = useState({ chat: '', embeddings: '', transcription: '' });
  const [busy, setBusy] = useState<'list' | 'save' | null>(null);
  const connectable = CONNECTABLE.find(c => c.provider === provider)!;

  const choose = (next: AiProvider) => {
    setProvider(next);
    setIds(null);
  };

  // Checks the key by asking for the provider's models, and picks one of each kind
  const list = async () => {
    setBusy('list');
    try {
      const models = provider === 'claude-code' ? [] : await settingsService.providerModels(provider, key.trim(), endpoint.trim());
      setIds(models);
      setChosen(pickModels(provider, models));
    } catch (error) {
      toast.error(t('connect.listFailed'), { description: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const use = async () => {
    setBusy('save');
    try {
      if (key.trim()) await settingsService.saveApiKey(provider, key.trim());
      const { keys: _keys, ...settings } = await settingsService.get();
      const searchHere = !chosen.embeddings;
      await settingsService.save({
        ...settings,
        provider,
        model: chosen.chat,
        customEndpoint: connectable.endpoint ? endpoint.trim() : settings.customEndpoint,
        // Without the provider's search, the built-in model (an Ollama one stays as it is)
        embeddingProvider: searchHere ? (settings.embeddingProvider === 'ollama' ? 'ollama' : 'builtin') : (provider as EmbeddingProvider),
        embeddingModel: searchHere ? (settings.embeddingProvider === 'ollama' ? settings.embeddingModel : BUILTIN_EMBEDDING) : chosen.embeddings,
      });
      const live = await liveService.status();
      await liveService.saveSettings({
        voiceConsent: live.voiceConsent,
        transcriptionModel: live.transcriptionModel,
        transcriptionProvider: chosen.transcription ? provider : 'builtin',
        transcriptionRemoteModel: chosen.transcription,
      });
      setKey('');
      const answer = await settingsService.test();
      toast.success(t('connect.done', { name: connectable.label }), { description: answer.slice(0, 120) });
      onConnected?.();
    } catch (error) {
      toast.error(t('connect.failed'), { description: String(error) });
      onConnected?.();
    } finally {
      setBusy(null);
    }
  };

  const options = (use: 'chat' | 'embeddings' | 'transcription', allowHere: boolean) => {
    const listed = modelsFor(ids ?? [], use).map(id => ({ value: id, label: id }));
    return allowHere ? [{ value: HERE, label: t('connect.here') }, ...listed] : listed;
  };

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="connect-provider">{t('connect.provider')}</Label>
          <SelectField
            id="connect-provider"
            value={provider}
            onValueChange={value => choose(value as AiProvider)}
            options={CONNECTABLE.map(c => ({ value: c.provider, label: c.provider === 'custom-openai' ? t('connect.custom') : c.label }))}
          />
        </div>
        {connectable.endpoint && (
          <div className="space-y-1.5">
            <Label htmlFor="connect-endpoint">{t('settings.ai.customEndpoint')}</Label>
            <Input id="connect-endpoint" value={endpoint} onChange={e => setEndpoint(e.target.value)} placeholder="http://localhost:8000/v1" />
          </div>
        )}
        {provider !== 'claude-code' && (
          <div className="space-y-1.5">
            <Label htmlFor="connect-key">{t('settings.ai.apiKey')}</Label>
            <Input id="connect-key" type="password" value={key} onChange={e => setKey(e.target.value)} placeholder={t(connectable.endpoint ? 'settings.ai.keyOptional' : 'connect.keyPlaceholder')} />
            {connectable.keyUrl && (
              <button onClick={() => openExternal(connectable.keyUrl!)} className="inline-flex items-center gap-1 text-xs text-blue-700 hover:underline">
                <ExternalLink className="h-3 w-3" /> {t('connect.getKey')}
              </button>
            )}
          </div>
        )}
      </div>
      {provider === 'claude-code' && <p className="text-xs text-gray-600">{t('settings.ai.claudeCodeHelp')}</p>}

      {ids === null ? (
        <Button onClick={list} disabled={busy !== null || (!key.trim() && !connectable.endpoint && provider !== 'claude-code')}>
          {busy === 'list' ? <ThinkingOrb variant="working" size={16} /> : <Plug className="h-4 w-4" />}
          {t('connect.check')}
        </Button>
      ) : (
        <div className="space-y-3 rounded-md border border-gray-200 p-3">
          <p className="text-xs text-gray-600">{t('connect.uses')}</p>
          <div className="grid gap-3 sm:grid-cols-3">
            <div className="space-y-1.5">
              <Label>{t('connect.chat')}</Label>
              {provider === 'claude-code' ? (
                <Input value={chosen.chat} onChange={e => setChosen({ ...chosen, chat: e.target.value })} />
              ) : (
                <SelectField value={chosen.chat} onValueChange={chat => setChosen({ ...chosen, chat })} options={options('chat', false)} placeholder={t('settings.ai.pickModel')} />
              )}
            </div>
            <div className="space-y-1.5">
              <Label>{t('connect.search')}</Label>
              {connectable.embeddings ? (
                <SelectField value={chosen.embeddings} onValueChange={embeddings => setChosen({ ...chosen, embeddings })} options={options('embeddings', true)} />
              ) : (
                <p className="text-sm text-gray-600">{t('connect.here')}</p>
              )}
            </div>
            <div className="space-y-1.5">
              <Label>{t('connect.transcription')}</Label>
              {connectable.transcription ? (
                <SelectField value={chosen.transcription} onValueChange={transcription => setChosen({ ...chosen, transcription })} options={options('transcription', true)} />
              ) : (
                <p className="text-sm text-gray-600">{t('connect.here')}</p>
              )}
            </div>
          </div>
          <p className="text-xs text-amber-800">{t('connect.privacy', { name: connectable.label })}</p>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setIds(null)} disabled={busy !== null}>
              {t('common.back')}
            </Button>
            <Button onClick={use} disabled={busy !== null || !chosen.chat}>
              {busy === 'save' && <ThinkingOrb variant="working" size={16} />}
              {t('connect.use', { name: connectable.label })}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
