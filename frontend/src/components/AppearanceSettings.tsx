'use client';

import { useEffect, useState } from 'react';
import { ThemePicker } from '@/components/ThemePicker';
import { Switch } from '@/components/ui/switch';
import { useI18n } from '@/i18n';

const TRANSPARENCY_KEY = 'app-transparency';

function SwitchCard({ title, help, checked, onChange }: { title: string; help: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
      <div className="flex items-center justify-between gap-4">
        <div>
          <h3 className="text-lg font-semibold text-gray-900 mb-2">{title}</h3>
          <p className="text-sm text-gray-600">{help}</p>
        </div>
        <Switch checked={checked} onCheckedChange={onChange} />
      </div>
    </div>
  );
}

/** Theme and (macOS) the window's transparency. */
export function AppearanceSettings() {
  const { t } = useI18n();
  // null: the window isn't translucent (not macOS), nothing to turn off
  const [transparent, setTransparent] = useState<boolean | null>(null);

  useEffect(() => {
    const root = document.documentElement;
    if (root.classList.contains('vibrancy')) setTransparent(!root.classList.contains('opaque'));
  }, []);

  // Read before first paint by THEME_BOOTSTRAP (app/layout.tsx)
  const saveTransparency = (on: boolean) => {
    setTransparent(on);
    document.documentElement.classList.toggle('opaque', !on);
    try {
      if (on) localStorage.removeItem(TRANSPARENCY_KEY);
      else localStorage.setItem(TRANSPARENCY_KEY, 'off');
    } catch {
      // Storage unavailable
    }
  };

  return (
    <div className="space-y-6">
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900 mb-2">{t('prefs.theme.title')}</h3>
        <p className="text-sm text-gray-600 mb-4">{t('prefs.theme.help')}</p>
        <ThemePicker />
      </div>

      {transparent !== null && (
        <SwitchCard title={t('prefs.transparency')} help={t('prefs.transparencyHelp')} checked={transparent} onChange={saveTransparency} />
      )}

    </div>
  );
}
