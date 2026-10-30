'use client';

import { useEffect, useState } from 'react';
import { LiveState, liveService } from '@/services/liveService';

/** The live session in progress, if any, kept current. */
export function useLiveState(): LiveState | null {
  const [state, setState] = useState<LiveState | null>(null);
  useEffect(() => {
    liveService
      .status()
      .then(status => setState(status.state))
      .catch(() => {});
    const unlisten = liveService.onState(setState);
    return () => {
      unlisten.then(fn => fn());
    };
  }, []);
  return state;
}
