'use client';

import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { Investigation, investigationService } from '@/services/investigationService';

const STORAGE_KEY = 'voga.context';
/** Where the open investigation was kept before contexts (read once, to keep it open). */
const LEGACY_KEY = 'voga.investigation';

/**
 * What is open: one investigation, several side by side, or the Journal. None on Home: the
 * sidebar shows only once something is open, with that context's pages.
 */
export type OpenContext = { kind: 'investigation'; ids: string[] } | { kind: 'journal' } | null;

interface InvestigationContextValue {
  investigations: Investigation[];
  context: OpenContext;
  /** The investigations open (one, or several compared). */
  scope: Investigation[];
  /** The investigation open when there is exactly one; the pages of a single investigation show it. */
  current: Investigation | null;
  loaded: boolean;
  open: (context: OpenContext) => void;
  /** Opens one investigation. */
  select: (id: string) => void;
  /** Reads the list again (after creating, renaming or deleting). */
  refresh: () => Promise<Investigation[]>;
}

const InvestigationContext = createContext<InvestigationContextValue | null>(null);

function saved(): OpenContext {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as OpenContext;
    const legacy = localStorage.getItem(LEGACY_KEY);
    return legacy ? { kind: 'investigation', ids: [legacy] } : null;
  } catch {
    return null;
  }
}

function keep(context: OpenContext) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(context));
  } catch {
    // This session only
  }
}

/** The context without the investigations that no longer exist. */
function valid(context: OpenContext, list: Investigation[]): OpenContext {
  if (context?.kind !== 'investigation') return context;
  const ids = context.ids.filter(id => list.some(i => i.id === id));
  return ids.length ? { kind: 'investigation', ids } : null;
}

export function InvestigationProvider({ children }: { children: React.ReactNode }) {
  const [investigations, setInvestigations] = useState<Investigation[]>([]);
  const [context, setContext] = useState<OpenContext>(null);
  const [loaded, setLoaded] = useState(false);

  const open = useCallback((next: OpenContext) => {
    setContext(next);
    keep(next);
  }, []);

  const select = useCallback((id: string) => open({ kind: 'investigation', ids: [id] }), [open]);

  const refresh = useCallback(async () => {
    const list = await investigationService.list();
    setInvestigations(list);
    setContext(previous => valid(loaded ? previous : saved(), list));
    setLoaded(true);
    return list;
  }, [loaded]);

  useEffect(() => {
    refresh().catch(error => {
      console.error('Failed to load investigations:', error);
      setLoaded(true);
    });
    // Once, at startup
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const scope = useMemo(
    () => (context?.kind === 'investigation' ? context.ids.map(id => investigations.find(i => i.id === id)).filter((i): i is Investigation => !!i) : []),
    [context, investigations],
  );
  const current = scope.length === 1 ? scope[0] : null;
  const value = useMemo(() => ({ investigations, context, scope, current, loaded, open, select, refresh }), [investigations, context, scope, current, loaded, open, select, refresh]);
  return <InvestigationContext.Provider value={value}>{children}</InvestigationContext.Provider>;
}

export function useInvestigation(): InvestigationContextValue {
  const context = useContext(InvestigationContext);
  if (!context) throw new Error('useInvestigation must be used within InvestigationProvider');
  return context;
}
