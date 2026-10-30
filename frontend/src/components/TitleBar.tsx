'use client';

import { useRouter } from 'next/navigation';
import { VogaMark } from '@/components/VogaMark';
import { useI18n } from '@/i18n';
import { cn } from '@/lib/utils';

/**
 * The window's top row, as tall as the band that drags it (MainContent): the
 * logo always sits here, right after the window buttons on macOS, whether the
 * row is the sidebar's top, Home's header or a page opened from Home.
 */
export function TitleBar({ children, className }: { children?: React.ReactNode; className?: string }) {
  const { t } = useI18n();
  const router = useRouter();
  return (
    <div className={cn('window-controls-gap flex h-11 flex-shrink-0 items-center justify-between gap-4 pl-4 pr-2', className)}>
      <button onClick={() => router.push('/')} className="flex items-center gap-2" title={t('nav.home')}>
        <VogaMark size={22} />
        <span className="text-base font-semibold tracking-tight text-gray-900">Voga</span>
      </button>
      {children}
    </div>
  );
}
