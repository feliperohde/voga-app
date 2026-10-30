'use client';

import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import { CheckCircle2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { VogaMark } from '@/components/VogaMark';
import { useUpdates } from '@/components/UpdateProvider';
import { UiLanguageSelect } from '@/i18n/UiLanguageSelect';
import { useI18n } from '@/i18n';

export function About() {
  const { t } = useI18n();
  const [version, setVersion] = useState('');
  const { checkNow, checking } = useUpdates();

  useEffect(() => {
    getVersion().then(setVersion).catch(console.error);
  }, []);

  return (
    <div className="space-y-6">
      <div className="space-y-3 rounded-lg border border-gray-200 bg-white p-6 text-center shadow-sm">
        <div className="flex justify-center">
          <VogaMark size={56} />
        </div>
        <div>
          <h3 className="text-xl font-semibold text-gray-900">Voga</h3>
          {version && <span className="text-sm text-gray-500">v{version}</span>}
        </div>
        <p className="text-sm text-gray-600">{t('about.tagline')}</p>
        <Button variant="outline" size="sm" onClick={checkNow} disabled={checking}>
          {checking ? <ThinkingOrb variant="working" size={14} className="mr-2" /> : <CheckCircle2 className="mr-2 h-3 w-3" />}
          {checking ? t('about.checking') : t('about.checkUpdates')}
        </Button>
        <p className="text-xs text-gray-500">{t('about.privacy')}</p>
      </div>
      <div className="rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
        <UiLanguageSelect />
      </div>
    </div>
  );
}
