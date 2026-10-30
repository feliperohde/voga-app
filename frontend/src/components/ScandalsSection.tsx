'use client';

import { useCallback, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Building2, Check, Siren, Trash2, X } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { useConfirm } from '@/components/ui/confirm-dialog';
import { formatDate, useI18n, type MessageKey } from '@/i18n';
import { claimsService } from '@/services/claimsService';
import { scandalService, type Involved, type Scandals, type Tie } from '@/services/scandalService';

const TIE: Record<string, MessageKey> = {
  SPENT_WITH: 'scandal.tie.SPENT_WITH',
  PAID_TO: 'scandal.tie.PAID_TO',
  FUNDS: 'scandal.tie.FUNDS',
  CONTRACTED: 'scandal.tie.CONTRACTED',
};

function TieLine({ tie }: { tie: Tie }) {
  const { t, locale } = useI18n();
  const label = TIE[tie.label] ? t(TIE[tie.label], { via: tie.via ?? '' }) : tie.label;
  // A monthly expense is dated by its month (2026-08), not a day
  const when = /^\d{4}-\d{2}$/.test(tie.date) ? tie.date.split('-').reverse().join('/') : tie.date && formatDate(locale, tie.date);
  return (
    <li className="text-xs text-gray-600">
      {[label, when].filter(Boolean).join(' · ')}
    </li>
  );
}

/** A political investigation's scandals (marked, or suggested by the AI) and the sanctioned companies its people are tied to. */
export function ScandalsSection({ investigationId, onChanged }: { investigationId: string; onChanged?: () => void }) {
  const { t, locale } = useI18n();
  const router = useRouter();
  const confirm = useConfirm();
  const [data, setData] = useState<Scandals | null>(null);

  const load = useCallback(() => scandalService.list(investigationId).then(setData), [investigationId]);
  useEffect(() => {
    load().catch(error => console.error('Failed to load scandals:', error));
  }, [load]);

  const review = (person: Involved, status: 'confirmed' | 'rejected') =>
    claimsService
      .setStatus(person.relationId, status)
      .then(load)
      .then(() => onChanged?.())
      .catch(error => toast.error(t('common.saveFailed'), { description: String(error) }));

  const remove = async (id: string) => {
    if (!(await confirm({ title: t('scandal.deleteTitle'), description: t('scandal.deleteHelp'), destructive: true, confirmLabel: t('common.delete') }))) return;
    scandalService
      .remove(id)
      .then(load)
      .then(() => onChanged?.())
      .catch(error => toast.error(String(error)));
  };

  if (!data) return null;
  return (
    <section className="rounded-lg border border-gray-200 bg-white shadow-sm">
      <div className="border-b border-gray-100 px-4 py-3">
        <h2 className="font-semibold text-gray-900">{t('scandal.title')}</h2>
        <p className="text-xs text-gray-500">{t('scandal.help')}</p>
      </div>
      {data.scandals.length === 0 && data.sanctioned.length === 0 && <p className="px-4 py-5 text-sm text-gray-500">{t('scandal.empty')}</p>}
      <ul className="divide-y divide-gray-100">
        {data.scandals.map(scandal => (
          <li key={scandal.id} className="flex items-start gap-3 px-4 py-3">
            <Siren className="mt-0.5 h-4 w-4 flex-shrink-0 text-red-700" />
            <div className="min-w-0 flex-1 space-y-1.5">
              <button
                onClick={() => scandal.documentId && router.push(`/document?id=${encodeURIComponent(scandal.documentId)}`)}
                className="block text-left font-medium text-gray-900 hover:underline"
              >
                {scandal.title}
              </button>
              <p className="text-xs text-gray-500">{[scandal.date && formatDate(locale, scandal.date), scandal.documentTitle].filter(Boolean).join(' · ')}</p>
              {scandal.people.map(person => (
                <div key={person.relationId} className="text-sm">
                  {person.status === 'suggested' ? (
                    <div className="space-y-1 rounded-md bg-amber-50 px-2.5 py-2">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium text-amber-900">{person.name}</span>
                        <span className="rounded bg-amber-100 px-1.5 py-px text-[11px] font-medium text-amber-900">{t('scandal.suggested')}</span>
                        <span className="ml-auto flex gap-1">
                          <Button size="sm" variant="outline" className="h-7" onClick={() => review(person, 'confirmed')}>
                            <Check className="mr-1 h-3.5 w-3.5" /> {t('scandal.confirm')}
                          </Button>
                          <Button size="sm" variant="ghost" className="h-7" onClick={() => review(person, 'rejected')}>
                            <X className="mr-1 h-3.5 w-3.5" /> {t('scandal.reject')}
                          </Button>
                        </span>
                      </div>
                      {person.rationale && <p className="text-xs italic text-amber-900">“{person.rationale}”</p>}
                    </div>
                  ) : (
                    <span className="text-gray-700">{t('scandal.involved', { name: person.name })}</span>
                  )}
                </div>
              ))}
            </div>
            <Button variant="ghost" size="icon" className="h-8 w-8 flex-shrink-0" title={t('common.delete')} onClick={() => remove(scandal.id)}>
              <Trash2 className="h-4 w-4" />
            </Button>
          </li>
        ))}
        {data.sanctioned.map(tie => (
          <li key={`${tie.targetId}-${tie.companyId}`} className="flex items-start gap-3 px-4 py-3">
            <Building2 className="mt-0.5 h-4 w-4 flex-shrink-0 text-gray-500" />
            <div className="min-w-0 flex-1 space-y-1">
              <p className="text-sm text-gray-900">{t('scandal.sanctionedTie', { person: tie.person, company: tie.company })}</p>
              <ul className="space-y-0.5">
                {tie.ties.slice(0, 3).map((line, i) => (
                  <TieLine key={i} tie={line} />
                ))}
              </ul>
              <ul className="space-y-0.5">
                {tie.sanctions.map((s, i) => (
                  <li key={i} className="text-xs text-red-800">
                    {[s.cadastro, s.categoria, s.orgao, [s.inicio, s.fim].filter(Boolean).join(' – ')].filter(Boolean).join(' · ')}
                  </li>
                ))}
              </ul>
            </div>
          </li>
        ))}
      </ul>
      {data.sanctioned.length > 0 && <p className="border-t border-gray-100 px-4 py-2.5 text-xs text-gray-500">{t('scandal.sanctionedNote')}</p>}
    </section>
  );
}
