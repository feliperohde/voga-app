'use client';

import { useEffect, useState } from 'react';
import { CollectProgress, investigationService } from '@/services/investigationService';

/** What the collection worker is doing, live. */
export function useCollectProgress(): CollectProgress | null {
  const [progress, setProgress] = useState<CollectProgress | null>(null);
  useEffect(() => {
    investigationService.collectStatus().then(setProgress).catch(error => console.error('Failed to read collection status:', error));
    const unlisten = investigationService.onProgress(setProgress);
    return () => {
      unlisten.then(fn => fn());
    };
  }, []);
  return progress;
}
