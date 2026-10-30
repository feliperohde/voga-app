'use client';

import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import type { Update } from '@tauri-apps/plugin-updater';
import { toast } from 'sonner';
import { UpdateDialog } from '@/components/UpdateDialog';
import { useI18n } from '@/i18n';
import { updateService } from '@/services/updateService';

const FIRST_LOOK_MS = 5_000;
const EVERY_MS = 24 * 60 * 60 * 1000;

interface Updates {
  /** Looks now (Settings › About, the tray menu): opens the new version, or says this is the latest. */
  checkNow: () => Promise<void>;
  checking: boolean;
}

const UpdatesContext = createContext<Updates>({ checkNow: async () => {}, checking: false });

export const useUpdates = () => useContext(UpdatesContext);

/** Looks for a new version a few seconds after Voga opens and once a day after that, quietly. */
export function UpdateProvider({ children }: { children: React.ReactNode }) {
  const { t } = useI18n();
  const [shown, setShown] = useState<Update | null>(null);
  const [checking, setChecking] = useState(false);
  const busy = useRef(false);
  const tr = useRef(t);
  tr.current = t;

  const look = useCallback(async (asked: boolean) => {
    const t = tr.current;
    if (busy.current) return;
    busy.current = true;
    setChecking(true);
    try {
      const update = await updateService.find();
      if (update && asked) setShown(update);
      else if (update) {
        toast(t('update.available', { version: update.version }), {
          duration: 15_000,
          action: { label: t('update.details'), onClick: () => setShown(update) },
        });
      } else if (asked) toast.success(t('about.latest'));
    } catch (error) {
      if (asked) toast.error(t('about.checkFailed'), { description: String(error) });
      else console.warn('update check failed', error);
    } finally {
      busy.current = false;
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    const first = setTimeout(() => look(false), FIRST_LOOK_MS);
    const daily = setInterval(() => look(false), EVERY_MS);
    // tray.rs: Procurar atualizações
    const fromTray = () => look(true);
    window.addEventListener('voga:check-updates', fromTray);
    return () => {
      clearTimeout(first);
      clearInterval(daily);
      window.removeEventListener('voga:check-updates', fromTray);
    };
  }, [look]);

  const checkNow = useCallback(() => look(true), [look]);

  return (
    <UpdatesContext.Provider value={{ checkNow, checking }}>
      {children}
      <UpdateDialog update={shown} onClose={() => setShown(null)} />
    </UpdatesContext.Provider>
  );
}
