'use client';

import { useCallback, useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Switch } from '@/components/ui/switch';
import { BuiltinModels } from '@/components/BuiltinModels';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useI18n } from '@/i18n';
import { SelectField } from '@/components/ui/select';
import { modelsFor } from '@/lib/providers';
import { builtinService } from '@/services/builtinService';
import { LiveStatus, liveService } from '@/services/liveService';
import { AiProvider, settingsService } from '@/services/settingsService';

/** Who can write the speech down: Whisper here, or a provider's transcription API. */
const TRANSCRIBERS: { value: string; label: string }[] = [
  { value: 'builtin', label: '' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'groq', label: 'Groq' },
  { value: 'custom-openai', label: '' },
];

/** The models live sessions use (Whisper, the voice model) and the consent to tell voices
 * apart. `onChange` hears every change, a download finished included. */
export function LiveSettings({ onChange }: { onChange?: (status: LiveStatus) => void }) {
  const { t } = useI18n();
  const [status, setStatus] = useState<LiveStatus | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [remoteModels, setRemoteModels] = useState<string[] | null>(null);
  const remote = status && status.transcriptionProvider !== 'builtin' ? status.transcriptionProvider : null;

  // The provider's transcription models (its key is the one saved in "Connect your AI")
  useEffect(() => {
    setRemoteModels(null);
    if (!remote) return;
    settingsService
      .providerModels(remote as AiProvider)
      .then(ids => setRemoteModels(modelsFor(ids, 'transcription')))
      .catch(error => {
        setRemoteModels([]);
        toast.error(t('connect.listFailed'), { description: String(error) });
      });
  }, [remote, t]);

  const reload = useCallback(
    () =>
      liveService
        .status()
        .then(status => {
          setStatus(status);
          onChange?.(status);
        })
        .catch(error => console.error('Failed to read the live settings:', error)),
    [onChange],
  );

  useEffect(() => {
    reload();
    const unlisten = builtinService.onProgress(update => {
      if (update.status === 'completed') reload();
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [reload]);

  if (!status) return <ThinkingOrb variant="working" size={20} />;

  const save = (changes: { voiceConsent?: boolean; transcriptionModel?: string; transcriptionProvider?: string; transcriptionRemoteModel?: string }) =>
    liveService
      .saveSettings({
        voiceConsent: changes.voiceConsent ?? status.voiceConsent,
        transcriptionModel: changes.transcriptionModel ?? chosen ?? status.transcriptionModel,
        transcriptionProvider: changes.transcriptionProvider,
        transcriptionRemoteModel: changes.transcriptionRemoteModel,
      })
      .then(reload)
      .catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));

  return (
    <div className="space-y-5">
      <div className="space-y-2">
        <h4 className="text-sm font-medium text-gray-900">{t('live.settings.transcription')}</h4>
        <SelectField
          value={status.transcriptionProvider}
          onValueChange={transcriptionProvider => save({ transcriptionProvider, transcriptionRemoteModel: '' })}
          aria-label={t('live.settings.transcription')}
          options={TRANSCRIBERS.map(o => ({
            value: o.value,
            label: o.value === 'builtin' ? t('live.settings.whisperHere') : o.value === 'custom-openai' ? t('connect.custom') : o.label,
          }))}
        />
        {remote ? (
          <div className="space-y-1.5">
            {remoteModels === null ? (
              <ThinkingOrb variant="working" size={16} />
            ) : (
              <SelectField
                value={status.transcriptionRemoteModel}
                onValueChange={transcriptionRemoteModel => save({ transcriptionRemoteModel })}
                options={[...new Set([...remoteModels, status.transcriptionRemoteModel].filter(Boolean))].map(id => ({ value: id, label: id }))}
                placeholder={t('settings.ai.pickModel')}
              />
            )}
            <p className="text-xs text-amber-800">{t('live.settings.remotePrivacy')}</p>
          </div>
        ) : (
          <BuiltinModels
            kind="transcription"
            selected={chosen ?? status.transcriptionModel ?? ''}
            onSelect={name => {
              setChosen(name);
              save({ transcriptionModel: name });
            }}
          />
        )}
      </div>
      <div className="space-y-2">
        <label className="flex items-start justify-between gap-4">
          <span>
            <span className="block text-sm font-medium text-gray-900">{t('live.settings.voices')}</span>
            <span className="block text-xs text-gray-600">{t('live.settings.voicesHelp')}</span>
          </span>
          <Switch checked={status.voiceConsent} onCheckedChange={voiceConsent => save({ voiceConsent })} aria-label={t('live.settings.voices')} />
        </label>
        {status.voiceConsent && <BuiltinModels kind="voice" selected="voice:wespeaker" onSelect={() => {}} />}
      </div>
    </div>
  );
}
