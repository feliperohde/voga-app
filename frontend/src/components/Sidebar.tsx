'use client';

import { Suspense, useState } from 'react';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { ArrowLeft, Check, ChevronsUpDown, GitCompareArrows, Hash, Plus, Settings, type LucideIcon } from 'lucide-react';
import { TitleBar } from '@/components/TitleBar';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { NewInvestigationDialog } from '@/components/NewInvestigationDialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { useCollectProgress } from '@/hooks/useCollectProgress';
import { useLiveState } from '@/hooks/useLiveState';
import { useI18n } from '@/i18n';
import { KIND_ICON, navPages } from '@/lib/nav';
import type { InvestigationKind } from '@/services/investigationService';

export const SIDEBAR_WIDTH = 220;

/** The sidebar shows once something is open, never on Home. */
export function useSidebarShown(): boolean {
  const pathname = usePathname() ?? '/';
  const { context } = useInvestigation();
  return context !== null && pathname !== '/';
}

function NavButton({ label, icon: Icon, active, onClick, recording }: { label: string; icon: LucideIcon; active: boolean; onClick: () => void; recording?: boolean }) {
  return (
    <button
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      className={`flex h-8 w-full items-center gap-2.5 rounded-md px-3 text-sm transition-colors ${
        active ? 'bg-gray-200/70 font-medium text-gray-900' : 'text-gray-600 hover:bg-gray-100 hover:text-gray-900'
      }`}
    >
      <Icon className="h-4 w-4 flex-shrink-0" />
      <span className="truncate">{label}</span>
      {recording && <span className="ml-auto h-2 w-2 flex-shrink-0 animate-pulse rounded-full bg-red-500" aria-label="●" />}
    </button>
  );
}

/** Switches what is open: an investigation, an interest or the Journal. */
function ContextPicker({ onNew }: { onNew: (kind: InvestigationKind) => void }) {
  const { t } = useI18n();
  const router = useRouter();
  const { investigations, context, scope, open } = useInvestigation();
  const label = context?.kind === 'journal' ? t('nav.journal') : scope.map(i => i.name).join(' × ');
  const caption =
    context?.kind === 'journal' ? t('home.journal') : scope.length > 1 ? t('compare.caption') : scope[0]?.kind === 'interest' ? t('investigation.kind.interest') : t('investigation.label');
  const others = investigations.filter(i => !scope.some(s => s.id === i.id));
  const group = (kind: InvestigationKind) => investigations.filter(i => i.kind === kind);
  const item = (id: string, name: string) => (
    <DropdownMenuItem
      key={id}
      onSelect={() => {
        open({ kind: 'investigation', ids: [id] });
        router.push('/investigation');
      }}
    >
      <Check className={`mr-2 h-4 w-4 ${scope.length === 1 && scope[0].id === id ? '' : 'invisible'}`} />
      <span className="truncate">{name}</span>
    </DropdownMenuItem>
  );
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button className="flex w-full items-center gap-2 rounded-md border border-gray-200 bg-white px-3 py-2 text-left text-sm shadow-sm hover:bg-gray-50">
          <span className="min-w-0 flex-1">
            <span className="block text-[11px] uppercase tracking-wide text-gray-500">{caption}</span>
            <span className="block truncate font-medium text-gray-900">{label}</span>
          </span>
          <ChevronsUpDown className="h-4 w-4 flex-shrink-0 text-gray-400" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-60">
        {scope.length > 0 && others.length > 0 && (
          <>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <GitCompareArrows className="mr-2 h-4 w-4" /> {t('compare.with')}
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="w-60">
                {others.map(other => {
                  const Icon = KIND_ICON[other.kind];
                  return (
                    <DropdownMenuItem
                      key={other.id}
                      onSelect={() => {
                        open({ kind: 'investigation', ids: [...scope.map(s => s.id), other.id] });
                        router.push('/compare');
                      }}
                    >
                      <Icon className="mr-2 h-4 w-4" />
                      <span className="truncate">{other.name}</span>
                    </DropdownMenuItem>
                  );
                })}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuSeparator />
          </>
        )}
        {group('politics').length > 0 && <DropdownMenuLabel className="text-xs text-gray-500">{t('home.investigations')}</DropdownMenuLabel>}
        {group('politics').map(i => item(i.id, i.name))}
        <DropdownMenuItem onSelect={() => onNew('politics')}>
          <Plus className="mr-2 h-4 w-4" /> {t('investigation.new')}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={() => {
            open({ kind: 'journal' });
            router.push('/journal');
          }}
        >
          <Check className={`mr-2 h-4 w-4 ${context?.kind === 'journal' ? '' : 'invisible'}`} />
          {t('nav.journal')}
        </DropdownMenuItem>
        {group('interest').map(i => item(i.id, i.name))}
        <DropdownMenuItem onSelect={() => onNew('interest')}>
          <Plus className="mr-2 h-4 w-4" /> {t('interest.new')}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function CollectStatus() {
  const { t } = useI18n();
  const progress = useCollectProgress();
  if (!progress?.running) return null;
  return (
    <div className="mx-1 flex items-center gap-2 rounded-md px-2 py-1.5 text-xs text-gray-600" title={progress.current ?? undefined}>
      <ThinkingOrb variant="background" size={16} />
      <span className="truncate">
        {progress.analyzing ? t('collect.analyzing') : progress.queued > 0 ? t('collect.runningQueued', { count: progress.queued }) : t('collect.running')}
      </span>
    </div>
  );
}

function Pages() {
  const { t } = useI18n();
  const pathname = usePathname() ?? '/';
  const search = useSearchParams();
  const router = useRouter();
  const { investigations, context, scope, open } = useInvestigation();
  const live = useLiveState();
  const isActive = (href: string) => {
    const [path, query] = href.split('?');
    if (!pathname.startsWith(path)) return false;
    // /journal and /journal?starred=1 are two entries
    const wanted = new URLSearchParams(query ?? '');
    return path !== '/journal' || (wanted.get('starred') === search.get('starred') && !search.get('id'));
  };
  const interests = investigations.filter(i => i.kind === 'interest');
  return (
    <nav className="flex-1 space-y-0.5 overflow-y-auto px-2">
      {navPages(context, scope)
        .map(page => (
          <NavButton
            key={page.href}
            label={t(page.label)}
            icon={page.icon}
            active={isActive(page.href)}
            onClick={() => router.push(page.href)}
            recording={live?.phase === 'recording' && page.href === (live.investigationId ? '/live' : '/debate')}
          />
        ))}
      {scope.length > 1 && (
        <>
          <div className="px-3 pb-1 pt-4 text-[11px] font-medium uppercase tracking-wide text-gray-500">{t('compare.open')}</div>
          {scope.map(investigation => (
            <NavButton
              key={investigation.id}
              label={investigation.name}
              icon={KIND_ICON[investigation.kind]}
              active={false}
              onClick={() => {
                open({ kind: 'investigation', ids: [investigation.id] });
                router.push('/investigation');
              }}
            />
          ))}
        </>
      )}
      {context?.kind === 'journal' && interests.length > 0 && (
        <>
          <div className="px-3 pb-1 pt-4 text-[11px] font-medium uppercase tracking-wide text-gray-500">{t('home.interests')}</div>
          {interests.map(interest => (
            <NavButton
              key={interest.id}
              label={interest.name}
              icon={Hash}
              active={pathname.startsWith('/journal') && search.get('id') === interest.id}
              onClick={() => router.push(`/journal?id=${encodeURIComponent(interest.id)}`)}
            />
          ))}
        </>
      )}
    </nav>
  );
}

export function Sidebar() {
  const { t } = useI18n();
  const pathname = usePathname() ?? '/';
  const router = useRouter();
  const shown = useSidebarShown();
  const [creating, setCreating] = useState<InvestigationKind | null>(null);
  if (!shown) return null;

  return (
    <aside className="app-sidebar fixed inset-y-0 left-0 z-20 flex flex-col border-r border-gray-200 bg-gray-50" style={{ width: SIDEBAR_WIDTH }}>
      <TitleBar />
      <div className="space-y-1 px-2 pb-3 pt-1">
        <NavButton label={t('nav.home')} icon={ArrowLeft} active={false} onClick={() => router.push('/')} />
        <ContextPicker onNew={setCreating} />
      </div>
      <Suspense fallback={<div className="flex-1" />}>
        <Pages />
      </Suspense>
      <div className="space-y-1 px-2 pb-3">
        <CollectStatus />
        <NavButton label={t('nav.settings')} icon={Settings} active={pathname.startsWith('/settings')} onClick={() => router.push('/settings')} />
      </div>
      <NewInvestigationDialog open={creating !== null} kind={creating ?? undefined} onOpenChange={open => !open && setCreating(null)} />
    </aside>
  );
}
