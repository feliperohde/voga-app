'use client';

import React from 'react';
import { Languages } from 'lucide-react';
import { LOCALES, LanguagePreference, useI18n } from '@/i18n';
import { SelectField } from '@/components/ui/select';

/** Picks the app's interface language. */
export function UiLanguageSelect({ compact = false }: { compact?: boolean }) {
  const { t, preference, setPreference } = useI18n();
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Languages className="h-4 w-4 text-gray-600" />
        <h4 className="text-sm font-medium text-gray-900">{t('uiLanguage.title')}</h4>
      </div>
      <SelectField
        value={preference}
        onValueChange={value => setPreference(value as LanguagePreference)}
        aria-label={t('uiLanguage.title')}
        options={[{ value: 'system', label: t('uiLanguage.system') }, ...LOCALES.map(locale => ({ value: locale.value, label: locale.label }))]}
      />
      {!compact && <p className="text-xs text-gray-500">{t('uiLanguage.help')}</p>}
    </div>
  );
}
