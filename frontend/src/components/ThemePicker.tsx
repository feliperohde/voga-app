'use client';

import { Check } from 'lucide-react';
import { Theme, useTheme } from '@/contexts/ThemeContext';
import { useI18n } from '@/i18n';
import type { MessageKey } from '@/i18n';
import { cn } from '@/lib/utils';

/** Each theme with the colors it is made of, shown as a small sample. */
const THEMES: { id: Theme; label: MessageKey; help: MessageKey; swatch: { page: string; surface: string; border: string; accent: string } }[] = [
  { id: 'light', label: 'prefs.theme.light', help: 'prefs.theme.lightHelp', swatch: { page: '#f9fafb', surface: '#ffffff', border: '#e5e7eb', accent: '#2563eb' } },
  { id: 'dark', label: 'prefs.theme.dark', help: 'prefs.theme.darkHelp', swatch: { page: '#0e1013', surface: '#16181d', border: '#2a2e36', accent: '#60a5fa' } },
  { id: 'neon', label: 'prefs.theme.neon', help: 'prefs.theme.neonHelp', swatch: { page: '#1c0320', surface: '#2b0730', border: '#39ff88', accent: '#ff4fd8' } },
];

export function ThemePicker() {
  const { t } = useI18n();
  const { theme, setTheme } = useTheme();
  return (
    <div role="radiogroup" aria-label={t('prefs.theme.title')} className="grid gap-3 sm:grid-cols-3">
      {THEMES.map(option => {
        const selected = theme === option.id;
        return (
          <button
            key={option.id}
            type="button"
            role="radio"
            aria-checked={selected}
            onClick={() => setTheme(option.id)}
            className={cn(
              'flex flex-col gap-2 rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              selected ? 'border-gray-900 bg-gray-50' : 'border-gray-200 hover:border-gray-400',
            )}
          >
            <span
              aria-hidden
              className="flex h-12 items-end gap-1.5 rounded-md p-1.5"
              style={{ background: option.swatch.page, border: `1px solid ${option.swatch.border}` }}
            >
              <span className="h-full w-1/3 rounded" style={{ background: option.swatch.surface, border: `1px solid ${option.swatch.border}` }} />
              <span className="h-1/2 flex-1 rounded" style={{ background: option.swatch.surface, border: `1px solid ${option.swatch.border}` }} />
              <span className="h-2.5 w-2.5 rounded-full" style={{ background: option.swatch.accent }} />
            </span>
            <span className="flex items-center justify-between gap-2 text-sm font-medium text-gray-900">
              {t(option.label)}
              {selected && <Check className="h-4 w-4" aria-hidden />}
            </span>
            <span className="text-xs text-gray-500">{t(option.help)}</span>
          </button>
        );
      })}
    </div>
  );
}
