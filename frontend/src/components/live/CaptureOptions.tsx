'use client';

import { Switch } from '@/components/ui/switch';
import { SelectField } from '@/components/ui/select';
import { useI18n } from '@/i18n';
import type { LiveStatus } from '@/services/liveService';

const DEFAULT_MIC = '';

/** What a new session hears, and whether it checks each claim as said. */
export interface Capture {
  mic: boolean;
  /** The microphone's name ("" for the default one). */
  device: string;
  system: boolean;
  autoCheck: boolean;
}

/** The computer's sound when it can be heard, else the microphone. */
export const initialCapture = (status: LiveStatus): Capture => ({ mic: !status.systemAudio, device: DEFAULT_MIC, system: status.systemAudio, autoCheck: true });

/** The microphone a session starts with (null: none). */
export const microphoneOf = (capture: Capture) => (capture.mic ? capture.device : null);

export function CaptureOptions({ status, value, onChange }: { status: LiveStatus; value: Capture; onChange: (value: Capture) => void }) {
  const { t } = useI18n();
  const set = (change: Partial<Capture>) => onChange({ ...value, ...change });
  return (
    <>
      <div className="space-y-3 rounded-md border border-gray-200 p-3">
        <label className="flex items-start justify-between gap-4">
          <span>
            <span className="block text-sm font-medium text-gray-900">{t('live.source.system')}</span>
            <span className="block text-xs text-gray-600">{t(status.systemAudio ? 'live.source.systemHelp' : 'live.source.systemUnavailable')}</span>
          </span>
          <Switch checked={value.system} disabled={!status.systemAudio} onCheckedChange={system => set({ system })} aria-label={t('live.source.system')} />
        </label>
        <label className="flex items-start justify-between gap-4">
          <span>
            <span className="block text-sm font-medium text-gray-900">{t('live.source.mic')}</span>
            <span className="block text-xs text-gray-600">{t('live.source.micHelp')}</span>
          </span>
          <Switch checked={value.mic} onCheckedChange={mic => set({ mic })} aria-label={t('live.source.mic')} />
        </label>
        {value.mic && status.microphones.length > 1 && (
          <SelectField
            value={value.device}
            onValueChange={device => set({ device })}
            aria-label={t('live.source.mic')}
            options={[{ value: DEFAULT_MIC, label: t('live.source.defaultMic') }, ...status.microphones.map(m => ({ value: m.name, label: m.name }))]}
          />
        )}
        {value.mic && value.system && <p className="text-xs text-amber-700">{t('live.source.both')}</p>}
      </div>
      <label className="flex items-start justify-between gap-4 rounded-md border border-gray-200 p-3">
        <span>
          <span className="block text-sm font-medium text-gray-900">{t('live.newAutoCheck')}</span>
          <span className="block text-xs text-gray-600">{t('live.newAutoCheckHelp')}</span>
        </span>
        <Switch checked={value.autoCheck} onCheckedChange={autoCheck => set({ autoCheck })} aria-label={t('live.newAutoCheck')} />
      </label>
    </>
  );
}
