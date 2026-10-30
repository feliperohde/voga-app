'use client';

import { useCallback, useEffect, useState } from 'react';
import { CheckCircle2, Download, Trash2, X } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Progress } from '@/components/ui/progress';
import { useConfirm } from '@/components/ui/confirm-dialog';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { MessageKey, useI18n } from '@/i18n';
import { cn } from '@/lib/utils';
import { BuiltinModel, BuiltinModelKind, builtinService, DownloadProgress, formatSize } from '@/services/builtinService';

/** The built-in models of one kind, kept current while they download. */
export function useBuiltinModels(kind: BuiltinModelKind) {
  const { t } = useI18n();
  const [models, setModels] = useState<BuiltinModel[] | null>(null);
  const [progress, setProgress] = useState<Record<string, DownloadProgress>>({});

  const reload = useCallback(
    () =>
      builtinService
        .list(kind)
        .then(setModels)
        .catch(error => console.error('Failed to list the built-in models:', error)),
    [kind],
  );

  useEffect(() => {
    reload();
    const unlisten = builtinService.onProgress(update => {
      setProgress(previous => ({ ...previous, [update.model]: update }));
      if (update.status !== 'downloading') reload();
    });
    return () => {
      unlisten.then(fn => fn());
    };
  }, [reload]);

  const download = useCallback(
    async (name: string) => {
      setProgress(previous => ({ ...previous, [name]: { model: name, progress: 0, status: 'downloading' } }));
      try {
        await builtinService.download(name);
      } catch (error) {
        // A cancelled download is not an error
        if (!String(error).startsWith('CANCELLED')) toast.error(t('builtin.downloadFailed'), { description: String(error) });
      } finally {
        setProgress(({ [name]: _done, ...rest }) => rest);
        reload();
      }
    },
    [reload, t],
  );

  /** Downloading now, with its percentage. */
  const downloading = (model: BuiltinModel): number | null => {
    const live = progress[model.name];
    if (live?.status === 'downloading') return live.progress;
    return model.status.type === 'downloading' ? model.status.progress : null;
  };

  return { models, progress, reload, download, downloading };
}

interface BuiltinModelsProps {
  kind: BuiltinModelKind;
  /** The model in use. */
  selected: string;
  onSelect: (name: string) => void;
}

/** Settings › AI: pick a built-in model, download it, cancel or delete it. */
export function BuiltinModels({ kind, selected, onSelect }: BuiltinModelsProps) {
  const { t } = useI18n();
  const confirm = useConfirm();
  const { models, progress, reload, download, downloading } = useBuiltinModels(kind);

  if (!models) return <ThinkingOrb variant="working" size={20} />;

  const remove = async (model: BuiltinModel) => {
    const ok = await confirm({
      title: t('builtin.deleteTitle', { name: model.display_name }),
      description: t('builtin.deleteHelp'),
      destructive: true,
      confirmLabel: t('common.delete'),
    });
    if (!ok) return;
    builtinService
      .remove(model.name)
      .catch(error => toast.error(String(error)))
      .finally(reload);
  };

  const selectedModel = models.find(m => m.name === selected);
  // The core's catalog is in English: the UI has each model's text in its own language
  const about = (model: BuiltinModel) => {
    const key = `builtin.about.${model.name}`;
    const text = t(key as MessageKey);
    return text === key ? model.description : text;
  };

  return (
    <div className="space-y-2">
      {models.map(model => {
        const percent = downloading(model);
        const live = progress[model.name];
        const available = model.status.type === 'available';
        const isSelected = model.name === selected;
        return (
          <div
            key={model.name}
            className={cn(
              'flex items-center gap-3 rounded-lg border p-3',
              isSelected ? 'border-blue-500 bg-blue-50/60' : 'border-gray-200 bg-white',
            )}
          >
            <button type="button" onClick={() => onSelect(model.name)} className="flex min-w-0 flex-1 items-start gap-3 text-left">
              <span
                className={cn(
                  'mt-1 h-4 w-4 flex-shrink-0 rounded-full border-2',
                  isSelected ? 'border-blue-600 bg-blue-600 shadow-[inset_0_0_0_2px_white]' : 'border-gray-300',
                )}
                aria-hidden
              />
              <span className="min-w-0">
                <span className="block text-sm font-medium text-gray-900">
                  {model.display_name} <span className="font-normal text-gray-500">· {formatSize(model.size_mb)}</span>
                </span>
                <span className="block text-xs text-gray-600">{about(model)}</span>
                {model.status.type === 'corrupted' && <span className="block text-xs text-red-600">{t('builtin.corrupted')}</span>}
              </span>
            </button>

            {percent !== null ? (
              <div className="flex w-44 flex-shrink-0 items-center gap-2">
                <div className="flex-1 space-y-1">
                  <Progress value={percent} />
                  <div className="text-[11px] text-gray-500">
                    {percent}%{live?.speed_mbps ? ` · ${t('builtin.speed', { speed: live.speed_mbps.toFixed(1) })}` : ''}
                  </div>
                </div>
                <Button variant="ghost" size="icon" onClick={() => builtinService.cancel(model.name).catch(() => {})} aria-label={t('builtin.cancel')}>
                  <X className="h-4 w-4" />
                </Button>
              </div>
            ) : available ? (
              <div className="flex flex-shrink-0 items-center gap-1">
                <span className="flex items-center gap-1 text-xs text-green-700">
                  <CheckCircle2 className="h-4 w-4" /> {t('builtin.downloaded')}
                </span>
                <Button variant="ghost" size="icon" onClick={() => remove(model)} aria-label={t('builtin.delete')}>
                  <Trash2 className="h-4 w-4 text-gray-500" />
                </Button>
              </div>
            ) : (
              <Button
                variant="outline"
                size="sm"
                className="flex-shrink-0"
                onClick={() => {
                  onSelect(model.name);
                  download(model.name);
                }}
              >
                <Download className="mr-2 h-4 w-4" />
                {t('builtin.download')}
              </Button>
            )}
          </div>
        );
      })}
      {selectedModel && selectedModel.status.type !== 'available' && downloading(selectedModel) === null && (
        <p className="text-xs text-amber-700">{t('builtin.notDownloaded')}</p>
      )}
    </div>
  );
}
