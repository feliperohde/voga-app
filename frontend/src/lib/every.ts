import type { Translate } from '@/i18n';

/** How often a source runs, as "a cada 6 h" or "diariamente"; 0 runs once. */
export function every(t: Translate, minutes: number): string {
  if (minutes === 0) return t('sources.once');
  if (minutes % 1440 === 0) return t('sources.everyDays', { count: minutes / 1440 });
  if (minutes % 60 === 0) return t('sources.everyHours', { count: minutes / 60 });
  return t('sources.everyMinutes', { count: minutes });
}
