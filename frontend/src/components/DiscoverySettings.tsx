'use client';

import { useEffect, useState } from 'react';
import { Sparkles } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useI18n } from '@/i18n';
import { Discovery, DiscoveryLimits, discoveryService } from '@/services/investigationService';

/** Sources › Discovery: how far the investigation looks by itself, and what it spent today. */
export function DiscoverySettings({ investigationId, onSuggested }: { investigationId: string; onSuggested: () => void }) {
  const { t } = useI18n();
  const [discovery, setDiscovery] = useState<Discovery | null>(null);
  const [limits, setLimits] = useState<DiscoveryLimits | null>(null);
  const [busy, setBusy] = useState<'save' | 'suggest' | null>(null);

  const load = () =>
    discoveryService
      .get(investigationId)
      .then(found => {
        setDiscovery(found);
        setLimits(found.limits);
      })
      .catch(error => console.error('Failed to load discovery settings:', error));

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [investigationId]);

  if (!discovery || !limits) return null;
  const set = (patch: Partial<DiscoveryLimits>) => setLimits({ ...limits, ...patch });
  const number = (value: string) => Math.max(0, Math.floor(Number(value) || 0));
  const changed = JSON.stringify(limits) !== JSON.stringify(discovery.limits);

  const save = async () => {
    setBusy('save');
    try {
      await discoveryService.save(investigationId, limits);
      await load();
      toast.success(t('common.saved'));
    } catch (error) {
      toast.error(t('common.saveFailed'), { description: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const suggest = async () => {
    setBusy('suggest');
    try {
      const count = await discoveryService.suggest(investigationId);
      toast.success(count > 0 ? t('discovery.suggested', { count }) : t('discovery.nothingSuggested'));
      onSuggested();
      await load();
    } catch (error) {
      toast.error(t('discovery.suggestFailed'), { description: String(error) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="space-y-4 px-5 py-4">
      <div className="grid gap-4 sm:grid-cols-3">
        <div className="space-y-1.5">
          <Label htmlFor="disc-depth">{t('discovery.depth')}</Label>
          <SelectField
            id="disc-depth"
            value={String(limits.depth)}
            onValueChange={value => set({ depth: Number(value) })}
            options={[
              { value: '0', label: t('discovery.depth0') },
              { value: '1', label: t('discovery.depth1') },
              { value: '2', label: t('discovery.depth2') },
            ]}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="disc-pages">{t('discovery.pagesPerDay')}</Label>
          <Input id="disc-pages" type="number" min={0} value={limits.pagesPerDay} onChange={e => set({ pagesPerDay: number(e.target.value) })} />
          <p className="text-xs text-gray-500">{t('discovery.usedToday', { used: discovery.usage.pages })}</p>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="disc-ai">{t('discovery.aiCallsPerDay')}</Label>
          <Input id="disc-ai" type="number" min={0} value={limits.aiCallsPerDay} onChange={e => set({ aiCallsPerDay: number(e.target.value) })} />
          <p className="text-xs text-gray-500">{t('discovery.usedToday', { used: discovery.usage.aiCalls })}</p>
        </div>
      </div>
      <div className="flex items-center justify-between gap-4">
        <div>
          <div className="text-sm font-medium text-gray-900">{t('discovery.autoApprove')}</div>
          <p className="text-xs text-gray-500">{t('discovery.autoApproveHelp')}</p>
        </div>
        <Switch checked={limits.autoApprove} onCheckedChange={autoApprove => set({ autoApprove })} />
      </div>
      <div className="flex flex-wrap justify-end gap-2">
        <Button variant="outline" onClick={suggest} disabled={!!busy}>
          {busy === 'suggest' ? <ThinkingOrb variant="thinking" size={16} className="mr-2" /> : <Sparkles className="mr-2 h-4 w-4" />}
          {t('discovery.suggestNow')}
        </Button>
        <Button onClick={save} disabled={!!busy || !changed}>
          {busy === 'save' && <ThinkingOrb variant="working" size={16} className="mr-2" />}
          {t('common.save')}
        </Button>
      </div>
    </div>
  );
}
