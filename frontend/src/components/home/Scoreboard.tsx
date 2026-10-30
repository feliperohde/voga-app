'use client';

import { useI18n } from '@/i18n';
import type { PersonScore } from '@/services/dashboardService';

const TOP = 5;

function Tile({ value, label, note }: { value: number; label: string; note?: string | null }) {
  const { locale } = useI18n();
  return (
    <div className="rounded-lg border border-gray-200 bg-white p-4 shadow-sm">
      <div className="text-4xl font-semibold tabular-nums tracking-tight text-gray-900">{value.toLocaleString(locale)}</div>
      <div className="mt-1 text-sm text-gray-600">{label}</div>
      {note && <div className="mt-0.5 text-xs text-gray-500">{note}</div>}
    </div>
  );
}

/**
 * Home's scoreboard: what a person confirmed about everyone followed (never the AI's suggestions,
 * counted apart), and who has the most confirmed contradictions and false statements.
 */
export function Scoreboard({ scores, onOpen }: { scores: PersonScore[]; onOpen: (score: PersonScore, href: string) => void }) {
  const { t } = useI18n();
  const sum = (pick: (s: PersonScore) => number) => scores.reduce((total, s) => total + pick(s), 0);
  const ranked = scores.filter(s => s.contradictions + s.falseChecks > 0).slice(0, TOP);
  const top = Math.max(1, ...ranked.map(s => s.contradictions + s.falseChecks));
  const sanctioned = sum(s => s.sanctioned);
  const toReview = sum(s => s.toReview);

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Tile value={sum(s => s.contradictions)} label={t('score.contradictions')} />
        <Tile value={sum(s => s.falseChecks)} label={t('score.falseChecks')} />
        <Tile value={sum(s => s.fulfilled + s.trueChecks)} label={t('score.truths')} note={t('score.truthsNote', { promises: sum(s => s.fulfilled), checks: sum(s => s.trueChecks) })} />
        <Tile value={sum(s => s.scandals)} label={t('score.scandals')} note={sanctioned > 0 ? t('score.sanctionedNote', { count: sanctioned }) : null} />
      </div>

      <div className="rounded-lg border border-gray-200 bg-white shadow-sm">
        <h3 className="border-b border-gray-100 px-4 py-3 font-semibold text-gray-900">{t('score.ranking')}</h3>
        {ranked.length === 0 ? (
          <p className="px-4 py-6 text-sm text-gray-500">{t('score.empty')}</p>
        ) : (
          <ol className="divide-y divide-gray-100">
            {ranked.map((score, i) => {
              const total = score.contradictions + score.falseChecks;
              return (
                <li key={score.targetId}>
                  <button onClick={() => onOpen(score, '/claims')} className="grid w-full grid-cols-[1.5rem_minmax(0,1fr)] items-center gap-3 px-4 py-3 text-left hover:bg-gray-50 sm:grid-cols-[1.5rem_minmax(0,14rem)_minmax(0,1fr)]">
                    <span className="text-sm tabular-nums text-gray-500">{i + 1}</span>
                    <span className="min-w-0">
                      <span className="block truncate font-medium text-gray-900">{score.name}</span>
                      <span className="block truncate text-xs text-gray-500">{score.investigationName}</span>
                    </span>
                    <span className="col-start-2 flex min-w-0 items-center gap-3 sm:col-start-3">
                      <span className="h-2 min-w-0 flex-1 rounded bg-gray-100" aria-hidden>
                        <span className="block h-full rounded bg-gray-700" style={{ width: `${(total / top) * 100}%` }} />
                      </span>
                      <span className="flex-shrink-0 text-xs text-gray-600">
                        {[
                          t('score.row.contradictions', { count: score.contradictions }),
                          t('score.row.false', { count: score.falseChecks }),
                          t('score.row.truths', { count: score.fulfilled + score.trueChecks }),
                          t('score.row.scandals', { count: score.scandals }),
                        ].join(' · ')}
                      </span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ol>
        )}
        <p className="border-t border-gray-100 px-4 py-2.5 text-xs text-gray-500">
          {toReview > 0 ? t('score.note', { count: toReview }) : t('score.noteNone')}
        </p>
      </div>
    </div>
  );
}
