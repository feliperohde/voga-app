'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { SelectField } from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useI18n, type MessageKey } from '@/i18n';
import { every } from '@/lib/every';
import { CollectSchedule, ScheduleEvery, scheduleService, ScheduleView } from '@/services/investigationService';

/** The paces offered, in minutes ('' keeps each source's own). */
const PACES = [30, 60, 180, 360, 720, 1440, 4320, 10080];
const ALL_DAYS = [0, 1, 2, 3, 4, 5, 6];
/** Groups an interest has no sources of. */
const POLITICS_ONLY: (keyof ScheduleEvery)[] = ['gazettes', 'official'];
const GROUPS: (keyof ScheduleEvery)[] = ['feeds', 'news', 'videos', 'pages', 'gazettes', 'official'];

/** Sources › Collection schedule: whether, how often and when the investigation collects. */
export function ScheduleSettings({ investigationId, interest, onSaved }: { investigationId: string; interest: boolean; onSaved: () => void }) {
  const { t, locale } = useI18n();
  const [view, setView] = useState<ScheduleView | null>(null);
  const [schedule, setSchedule] = useState<CollectSchedule | null>(null);
  const [saving, setSaving] = useState(false);

  const load = () =>
    scheduleService
      .get(investigationId)
      .then(found => {
        setView(found);
        setSchedule(found.schedule);
      })
      .catch(error => console.error('Failed to load the collection schedule:', error));

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [investigationId]);

  if (!view || !schedule) return null;
  const set = (patch: Partial<CollectSchedule>) => setSchedule({ ...schedule, ...patch });
  const changed = JSON.stringify(schedule) !== JSON.stringify(view.schedule);
  const days = schedule.days.length ? schedule.days : ALL_DAYS;
  const weekday = (day: number) => new Intl.DateTimeFormat(locale, { weekday: 'short' }).format(new Date(2026, 0, 4 + day)); // 4 Jan 2026 is a Sunday

  const toggleDay = (day: number) => {
    const next = days.includes(day) ? days.filter(d => d !== day) : [...days, day].sort((a, b) => a - b);
    // No day at all would be a pause; the switch above says so plainly
    if (next.length === 0) return;
    set({ days: next.length === 7 ? [] : next });
  };

  const save = async () => {
    setSaving(true);
    try {
      await scheduleService.save(investigationId, schedule);
      await load();
      onSaved();
      toast.success(t('common.saved'));
    } catch (error) {
      toast.error(t('common.saveFailed'), { description: String(error) });
    } finally {
      setSaving(false);
    }
  };

  const status: MessageKey = view.open ? 'schedule.open' : view.schedule.paused ? 'schedule.closedPaused' : 'schedule.closedWindow';

  return (
    <div className="space-y-5 px-5 py-4">
      <div className="flex items-center justify-between gap-4">
        <div>
          <div className="text-sm font-medium text-gray-900">{t('schedule.paused')}</div>
          <p className="text-xs text-gray-500">{t('schedule.pausedHelp')}</p>
        </div>
        <Switch checked={schedule.paused} onCheckedChange={paused => set({ paused })} aria-label={t('schedule.paused')} />
      </div>

      <div className="space-y-2">
        <div className="text-sm font-medium text-gray-900">{t('schedule.frequency')}</div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {GROUPS.filter(group => !(interest && POLITICS_ONLY.includes(group))).map(group => (
            <div key={group} className="space-y-1.5">
              <Label htmlFor={`pace-${group}`}>{t(`schedule.group.${group}`)}</Label>
              <SelectField
                id={`pace-${group}`}
                value={schedule.every[group] === null ? '' : String(schedule.every[group])}
                onValueChange={value => set({ every: { ...schedule.every, [group]: value === '' ? null : Number(value) } })}
                options={[{ value: '', label: t('schedule.default') }, ...PACES.map(minutes => ({ value: String(minutes), label: every(t, minutes) }))]}
              />
              <p className="text-xs text-gray-500">{t(`schedule.default.${group}`)}</p>
              {group === 'official' && <p className="text-xs text-gray-500">{t('schedule.officialHelp')}</p>}
            </div>
          ))}
        </div>
      </div>

      <div className="space-y-2">
        <Label htmlFor="schedule-when">{t('schedule.when')}</Label>
        <div className="flex flex-wrap items-end gap-3">
          <SelectField
            id="schedule-when"
            className="w-48"
            value={schedule.window ? 'window' : 'any'}
            onValueChange={value => set({ window: value === 'window' ? { start: '22:00', end: '06:00' } : null })}
            options={[
              { value: 'any', label: t('schedule.anyTime') },
              { value: 'window', label: t('schedule.window') },
            ]}
          />
          {schedule.window && (
            <>
              <div className="space-y-1.5">
                <Label htmlFor="schedule-from">{t('schedule.from')}</Label>
                <Input id="schedule-from" type="time" className="w-32" value={schedule.window.start} onChange={e => e.target.value && set({ window: { ...schedule.window!, start: e.target.value } })} />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="schedule-to">{t('schedule.to')}</Label>
                <Input id="schedule-to" type="time" className="w-32" value={schedule.window.end} onChange={e => e.target.value && set({ window: { ...schedule.window!, end: e.target.value } })} />
              </div>
            </>
          )}
        </div>
        {schedule.window && <p className="text-xs text-gray-500">{t('schedule.overnight')}</p>}
      </div>

      <div className="space-y-2">
        <div className="text-sm font-medium text-gray-900">{t('schedule.days')}</div>
        <div className="flex flex-wrap gap-1.5">
          {ALL_DAYS.map(day => (
            <Button key={day} type="button" size="sm" variant={days.includes(day) ? 'default' : 'outline'} aria-pressed={days.includes(day)} onClick={() => toggleDay(day)} className="min-w-12 capitalize">
              {weekday(day)}
            </Button>
          ))}
        </div>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className={view.open ? 'text-xs text-gray-500' : 'text-xs font-medium text-amber-700'}>{t(status)}</p>
        <Button onClick={save} disabled={saving || !changed}>
          {saving && <ThinkingOrb variant="working" size={16} className="mr-2" />}
          {t('common.save')}
        </Button>
      </div>
    </div>
  );
}
