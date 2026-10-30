'use client';

import React, { createContext, useContext, useState } from 'react';
import { createPortal } from 'react-dom';
import { useRouter } from 'next/navigation';
import { ArrowLeft, Home } from 'lucide-react';
import { useSidebarShown } from '@/components/Sidebar';
import { TitleBar } from '@/components/TitleBar';
import { useI18n } from '@/i18n';
import { cn } from '@/lib/utils';

/**
 * Page-level actions (Save) always live at the top-right of the page header.
 * Forms deep inside a page (e.g. a settings tab) render <PageActions> and the
 * buttons are portaled into the header; outside a PageActionsProvider they
 * render inline, so the same form also works inside a modal.
 */
interface PageActionsContextValue {
  slot: HTMLElement | null;
  setSlot: (element: HTMLElement | null) => void;
}

const PageActionsContext = createContext<PageActionsContextValue | null>(null);

export function PageActionsProvider({ children }: { children: React.ReactNode }) {
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  return (
    <PageActionsContext.Provider value={{ slot, setSlot }}>
      {children}
    </PageActionsContext.Provider>
  );
}

interface PageActionsProps {
  children: React.ReactNode;
  /** Wrapper class used only when rendered inline (no page header to portal into). */
  inlineClassName?: string;
}

export function PageActions({ children, inlineClassName }: PageActionsProps) {
  const context = useContext(PageActionsContext);
  if (!context) return <div className={inlineClassName}>{children}</div>;
  if (!context.slot) return null;
  return createPortal(children, context.slot);
}

interface PageHeaderProps {
  /** Wraps onto more lines when long; a page that shows it elsewhere (a document) leaves it out. */
  title?: React.ReactNode;
  onBack?: () => void;
  /** Static actions owned by the page itself; form actions arrive via <PageActions>. */
  actions?: React.ReactNode;
  /** Pin the actions to the window's top-right corner. */
  actionsInCorner?: boolean;
  className?: string;
}

export function PageHeader({ title, onBack, actions, actionsInCorner = false, className }: PageHeaderProps) {
  const { t } = useI18n();
  const router = useRouter();
  const context = useContext(PageActionsContext);
  // Without the sidebar (a page opened from Home), the logo's row and the way back are here
  const home = !useSidebarShown();

  return (
    <div className="sticky top-0 z-10 bg-white border-b border-gray-200">
      {home && <TitleBar />}
      <div className={cn('relative mx-auto flex items-center justify-between gap-4 px-4 py-6 sm:px-8', home && 'pt-2', actionsInCorner && 'pr-40 sm:pr-48', className)}>
        <div className="flex min-w-0 items-center gap-4">
          {home && !onBack && (
            <button onClick={() => router.push('/')} className="flex flex-shrink-0 items-center gap-2 text-gray-600 transition-colors hover:text-gray-900">
              <Home className="h-5 w-5" />
              <span>{t('nav.home')}</span>
            </button>
          )}
          {onBack && (
            <button
              onClick={onBack}
              className="flex flex-shrink-0 items-center gap-2 text-gray-600 hover:text-gray-900 transition-colors"
            >
              <ArrowLeft className="w-5 h-5" />
              <span>{t('common.back')}</span>
            </button>
          )}
          {title && <h1 className="min-w-0 break-words text-3xl font-bold">{title}</h1>}
        </div>
        <div
          className={cn(
            'flex flex-shrink-0 items-center gap-2',
            actionsInCorner && 'absolute right-4 top-1/2 -translate-y-1/2 animate-in fade-in zoom-in-95 sm:right-8',
          )}
        >
          {actions}
          {context && <div ref={context.setSlot} className="flex items-center gap-2" />}
        </div>
      </div>
    </div>
  );
}
