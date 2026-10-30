'use client';

import { useEffect, useState } from 'react';
import type { Update } from '@tauri-apps/plugin-updater';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Progress } from '@/components/ui/progress';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { formatDate, useI18n } from '@/i18n';
import { formatSize } from '@/services/builtinService';
import { updateService, type InstallStep } from '@/services/updateService';

const mb = (bytes: number) => formatSize(bytes / (1024 * 1024));

/** The new version's notes and the button that installs it; it cannot be closed while installing. */
export function UpdateDialog({ update, onClose }: { update: Update | null; onClose: () => void }) {
  const { locale, t } = useI18n();
  const [step, setStep] = useState<InstallStep | null>(null);

  useEffect(() => setStep(null), [update]);

  const install = async () => {
    if (!update) return;
    setStep({ phase: 'download', received: 0, total: 0 });
    try {
      await updateService.install(update, setStep);
    } catch (error) {
      setStep(null);
      toast.error(t('update.failed'), { description: String(error) });
    }
  };

  const busy = step !== null;

  return (
    <Dialog open={update !== null} onOpenChange={open => !open && !busy && onClose()}>
      <DialogContent className="sm:max-w-lg" onEscapeKeyDown={e => busy && e.preventDefault()} onInteractOutside={e => busy && e.preventDefault()}>
        <DialogHeader>
          <DialogTitle>{t('update.title', { version: update?.version ?? '' })}</DialogTitle>
          <DialogDescription>
            {t('update.current', { version: update?.currentVersion ?? '' })}
            {update?.date && ` ${t('update.published', { date: formatDate(locale, update.date, { dateStyle: 'long' }) })}`}
          </DialogDescription>
        </DialogHeader>

        {step === null && update?.body && (
          <div className="prose prose-sm max-h-72 max-w-none select-text overflow-y-auto rounded-md border p-3 dark:prose-invert">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{update.body}</ReactMarkdown>
          </div>
        )}

        {step?.phase === 'download' && (
          <div className="space-y-2">
            <Progress value={step.total > 0 ? (100 * step.received) / step.total : 0} />
            <p className="text-sm text-muted-foreground">
              {step.total > 0
                ? t('update.downloading', { received: mb(step.received), total: mb(step.total) })
                : t('update.downloadingUnknown', { received: mb(step.received) })}
            </p>
          </div>
        )}
        {step?.phase === 'install' && (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <ThinkingOrb variant="background" size={16} />
            {t('update.installing')}
          </p>
        )}

        {!busy && (
          <DialogFooter>
            <Button variant="outline" onClick={onClose}>
              {t('update.later')}
            </Button>
            <Button onClick={install}>{t('update.install')}</Button>
          </DialogFooter>
        )}
      </DialogContent>
    </Dialog>
  );
}
