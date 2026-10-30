'use client';

import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { useI18n } from '@/i18n';
import { investigationService, type Target, type VogaDocument } from '@/services/investigationService';
import { scandalService } from '@/services/scandalService';

/** Marks a news item as a scandal and says who, of the people followed, it involves. */
export function ScandalDialog({ document, open, onOpenChange }: { document: VogaDocument; open: boolean; onOpenChange: (open: boolean) => void }) {
  const { t } = useI18n();
  const [people, setPeople] = useState<Target[]>([]);
  const [title, setTitle] = useState(document.title);
  const [date, setDate] = useState((document.publishedAt ?? document.fetchedAt).slice(0, 10));
  const [chosen, setChosen] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    investigationService
      .targets(document.investigationId)
      .then(targets => {
        const persons = targets.filter(target => target.kind === 'person');
        setPeople(persons);
        // The one person followed is the one involved, unless said otherwise
        setChosen(persons.length === 1 ? [persons[0].id] : []);
      })
      .catch(error => console.error('Failed to load the people followed:', error));
  }, [open, document.investigationId]);

  const save = async () => {
    setSaving(true);
    try {
      await scandalService.mark(document.id, title.trim(), date, chosen);
      toast.success(t('scandal.marked'));
      onOpenChange(false);
    } catch (error) {
      toast.error(t('common.saveFailed'), { description: String(error) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t('scandal.mark')}</DialogTitle>
          <DialogDescription>{t('scandal.markHelp')}</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="scandal-title">{t('scandal.name')}</Label>
            <Input id="scandal-title" value={title} onChange={e => setTitle(e.target.value)} />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="scandal-date">{t('scandal.date')}</Label>
            <Input id="scandal-date" type="date" className="w-44" value={date} onChange={e => setDate(e.target.value)} />
          </div>
          <fieldset className="space-y-2">
            <legend className="text-sm font-medium text-gray-900">{t('scandal.people')}</legend>
            {people.length === 0 && <p className="text-sm text-gray-500">{t('scandal.noPeople')}</p>}
            {people.map(person => (
              <label key={person.id} className="flex items-center gap-2 text-sm text-gray-800">
                <Checkbox
                  checked={chosen.includes(person.id)}
                  onCheckedChange={checked => setChosen(checked ? [...chosen, person.id] : chosen.filter(id => id !== person.id))}
                />
                {person.name}
              </label>
            ))}
          </fieldset>
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t('common.cancel')}
          </Button>
          <Button onClick={save} disabled={saving || !title.trim() || chosen.length === 0}>
            {saving && <ThinkingOrb variant="working" size={16} className="mr-2" />}
            {t('scandal.mark')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
