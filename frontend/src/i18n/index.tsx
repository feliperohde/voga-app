'use client';

/**
 * App (UI) language.
 * Messages live in ./messages; English is the fallback for any missing key.
 * The preference is per device (localStorage); "system" follows the OS locale.
 */

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { en, MessageKey } from './messages/en';
import { ptBR } from './messages/pt-BR';

export type Locale = 'en' | 'pt-BR';
export type LanguagePreference = 'system' | Locale;
export type { MessageKey };

export const LOCALES: { value: Locale; label: string }[] = [
  { value: 'en', label: 'English' },
  { value: 'pt-BR', label: 'Português (Brasil)' },
];

const MESSAGES: Record<Locale, Record<MessageKey, string>> = { en, 'pt-BR': ptBR };
const STORAGE_KEY = 'voga.uiLanguage';

export type TranslateVars = Record<string, string | number>;
export type Translate = (key: MessageKey, vars?: TranslateVars) => string;

function systemLocale(): Locale {
  if (typeof navigator === 'undefined') return 'en';
  const languages = navigator.languages?.length ? navigator.languages : [navigator.language];
  return languages.some(l => l?.toLowerCase().startsWith('pt')) ? 'pt-BR' : 'en';
}

export function resolveLocale(preference: LanguagePreference): Locale {
  return preference === 'system' ? systemLocale() : preference;
}

export function translate(locale: Locale, key: MessageKey, vars?: TranslateVars): string {
  // "<key>.one", when it exists, is the singular of a message with {count}
  const one = vars?.count === 1 ? (`${key}.one` as MessageKey) : null;
  const template = (one && (MESSAGES[locale][one] ?? en[one])) || MESSAGES[locale][key] || en[key] || key;
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (match, name) => (name in vars ? String(vars[name]) : match));
}

interface I18nContextValue {
  locale: Locale;
  /** The user's language is known. Before (first render, and child effects on mount) it is English:
   *  text made once at startup, like a toast, waits for this. */
  ready: boolean;
  preference: LanguagePreference;
  setPreference: (preference: LanguagePreference) => void;
  t: Translate;
}

const I18nContext = createContext<I18nContextValue | null>(null);

function readPreference(): LanguagePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'system' || stored === 'en' || stored === 'pt-BR') return stored;
  } catch {
    // Storage unavailable: follow the system
  }
  return 'system';
}

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<LanguagePreference>('system');
  const [locale, setLocale] = useState<Locale>('en');
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const stored = readPreference();
    setPreferenceState(stored);
    setLocale(resolveLocale(stored));
    setReady(true);
  }, []);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  const setPreference = useCallback((next: LanguagePreference) => {
    setPreferenceState(next);
    setLocale(resolveLocale(next));
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Applies for this session only
    }
  }, []);

  const t = useCallback<Translate>((key, vars) => translate(locale, key, vars), [locale]);
  const value = useMemo(() => ({ locale, ready, preference, setPreference, t }), [locale, ready, preference, setPreference, t]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  const context = useContext(I18nContext);
  if (!context) {
    // Outside the provider (tests, isolated renders): English
    return { locale: 'en', ready: true, preference: 'en', setPreference: () => {}, t: (key, vars) => translate('en', key, vars) };
  }
  return context;
}

/** Date formatting in the UI language; options asking for the time (timeStyle,
 *  hour…) include it, which toLocaleDateString refuses to do. */
export function formatDate(locale: Locale, value: string | Date, options?: Intl.DateTimeFormatOptions): string {
  // A bare date (2022-10-02) is that day where the user is, not midnight UTC (the day before in Brazil)
  const day = typeof value === 'string' ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(value) : null;
  const date = day ? new Date(Number(day[1]), Number(day[2]) - 1, Number(day[3])) : typeof value === 'string' ? new Date(value) : value;
  if (Number.isNaN(date.getTime())) return String(value);
  const withTime = !!options && ['timeStyle', 'hour', 'minute', 'second'].some(key => key in options);
  return withTime ? date.toLocaleString(locale, options) : date.toLocaleDateString(locale, options);
}

/** How long ago, in the UI language: "há 5 minutos", "ontem", "há 3 dias". */
export function formatAgo(locale: Locale, value: string): string {
  const seconds = (new Date(value).getTime() - Date.now()) / 1000;
  if (Number.isNaN(seconds)) return value;
  const format = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [['minute', 60], ['hour', 3600], ['day', 86400], ['month', 2592000], ['year', 31536000]];
  const [unit, size] = [...steps].reverse().find(([, size]) => Math.abs(seconds) >= size) ?? steps[0];
  return format.format(Math.round(seconds / size), unit);
}
