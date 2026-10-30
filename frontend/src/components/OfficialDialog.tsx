'use client';

import { useCallback, useEffect, useState } from 'react';
import { Landmark } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useI18n } from '@/i18n';
import { investigationService, OfficialMatch, Target } from '@/services/investigationService';

const key = (m: OfficialMatch) => `${m.kind}:${m.id}`;

/**
 * Finds the target in the Câmara, the Senado and the TSE and links the records the user
 * confirms. Nothing is linked on its own: people share names.
 */
export function OfficialPicker({ target, onLinked, onCancel }: { target: Target; onLinked: () => void; onCancel?: () => void }) {
  const { t } = useI18n();
  const [matches, setMatches] = useState<OfficialMatch[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [linked, setLinked] = useState<Set<string>>(new Set());
  const [municipal, setMunicipal] = useState(false);
  const [linking, setLinking] = useState(false);
  const targetId = target.id;

  const search = useCallback(
    (withMunicipal: boolean) => {
      setMatches(null);
      setError(null);
      setChosen(new Set());
      investigationService
        .findOfficial(targetId, withMunicipal ? [2022, 2024, 2026] : undefined)
        .then(setMatches)
        .catch(e => setError(String(e)));
    },
    [targetId],
  );

  useEffect(() => {
    setMunicipal(false);
    search(false);
  }, [search]);
  useEffect(() => setLinked(new Set(target.official.map(key))), [target]);

  const toggle = (match: OfficialMatch) => {
    const next = new Set(chosen);
    if (next.has(key(match))) next.delete(key(match));
    else next.add(key(match));
    setChosen(next);
  };

  const row = (match: OfficialMatch) => {
    const already = linked.has(key(match));
    return (
      <label key={key(match)} className={`flex items-start gap-3 rounded-md px-2 py-2 ${already ? 'opacity-60' : 'cursor-pointer hover:bg-gray-50'}`}>
        <Checkbox checked={already || chosen.has(key(match))} disabled={already} onCheckedChange={() => toggle(match)} className="mt-0.5" />
        <span className="min-w-0">
          <span className="block font-medium text-gray-900">{match.label}</span>
          <span className="block text-xs text-gray-500">
            {match.detail}
            {already && ` · ${t('official.alreadyLinked')}`}
          </span>
        </span>
      </label>
    );
  };

  const link = async () => {
    if (!matches) return;
    setLinking(true);
    try {
      await investigationService.linkOfficial(targetId, matches.filter(m => chosen.has(key(m))));
      toast.success(t('official.linked'));
      setLinked(new Set([...linked, ...chosen]));
      setChosen(new Set());
      onLinked();
    } catch (e) {
      toast.error(t('official.linkFailed'), { description: String(e) });
    } finally {
      setLinking(false);
    }
  };

  return (
    <div className="space-y-4">
      {error ? (
        <p className="text-sm text-red-600">{error}</p>
      ) : matches === null ? (
        <div className="flex items-center gap-2 py-6 text-sm text-gray-600">
          <ThinkingOrb variant="background" size={20} /> {t('official.searching')}
        </div>
      ) : matches.length === 0 ? (
        <p className="py-4 text-sm text-gray-600">{t('official.none')}</p>
      ) : (
        <div className="max-h-80 space-y-1 overflow-y-auto">
          {matches.filter(m => !m.similar).map(row)}
          {matches.some(m => m.similar) && (
            <details className="pt-1">
              <summary className="cursor-pointer px-2 py-1 text-xs text-gray-500">{t('official.similar', { count: matches.filter(m => m.similar).length })}</summary>
              {matches.filter(m => m.similar).map(row)}
            </details>
          )}
        </div>
      )}

      <label className="flex items-center gap-2 text-xs text-gray-600">
        <Checkbox
          checked={municipal}
          onCheckedChange={checked => {
            setMunicipal(!!checked);
            search(!!checked);
          }}
        />
        {t('official.municipal')}
      </label>

      <div className="flex justify-end gap-2">
        {onCancel && (
          <Button variant="outline" onClick={onCancel}>
            {t('common.cancel')}
          </Button>
        )}
        <Button onClick={link} disabled={linking || chosen.size === 0}>
          {linking && <ThinkingOrb variant="working" size={16} className="mr-2" />}
          {t('official.link', { count: chosen.size })}
        </Button>
      </div>
    </div>
  );
}

export function OfficialDialog({ target, onClose, onLinked }: { target: Target | null; onClose: () => void; onLinked: () => void }) {
  const { t } = useI18n();
  // The last target stays while the dialog fades out
  const [shown, setShown] = useState<Target | null>(target);
  useEffect(() => {
    if (target) setShown(target);
  }, [target]);

  return (
    <Dialog open={target !== null} onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Landmark className="h-5 w-5 text-gray-600" /> {t('official.title', { name: shown?.name ?? '' })}
          </DialogTitle>
          <DialogDescription>{t('official.help')}</DialogDescription>
        </DialogHeader>
        {shown && (
          <OfficialPicker
            key={shown.id}
            target={shown}
            onCancel={onClose}
            onLinked={() => {
              onLinked();
              onClose();
            }}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}
