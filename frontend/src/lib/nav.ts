import { AudioLines, FileText, GitCompareArrows, Hash, Landmark, LayoutDashboard, MessageSquareText, Network, Newspaper, Radio, Scale, Star, Swords, type LucideIcon } from 'lucide-react';
import type { OpenContext } from '@/contexts/InvestigationContext';
import type { MessageKey } from '@/i18n';
import type { Investigation, InvestigationKind } from '@/services/investigationService';

export interface NavPage {
  label: MessageKey;
  href: string;
  icon: LucideIcon;
}

const POLITICS: NavPage[] = [
  { label: 'nav.overview', href: '/investigation', icon: LayoutDashboard },
  { label: 'nav.news', href: '/feed', icon: Newspaper },
  { label: 'nav.sources', href: '/sources', icon: Radio },
  { label: 'nav.documents', href: '/documents', icon: FileText },
  { label: 'nav.claims', href: '/claims', icon: Scale },
  { label: 'nav.live', href: '/live', icon: AudioLines },
  // Debates are outside every investigation, but only make sense next to politics
  { label: 'nav.debate', href: '/debate', icon: Swords },
  { label: 'nav.map', href: '/map', icon: Network },
  { label: 'nav.ask', href: '/ask', icon: MessageSquareText },
];

const INTEREST: NavPage[] = [
  { label: 'nav.overview', href: '/investigation', icon: LayoutDashboard },
  { label: 'nav.news', href: '/feed', icon: Newspaper },
  { label: 'nav.sources', href: '/sources', icon: Radio },
  { label: 'nav.documents', href: '/documents', icon: FileText },
  { label: 'nav.ask', href: '/ask', icon: MessageSquareText },
];

/** Several open together: what works across them (each one's other pages open it alone). */
const COMPARE: NavPage[] = [
  { label: 'nav.compare', href: '/compare', icon: GitCompareArrows },
  { label: 'nav.news', href: '/feed', icon: Newspaper },
  { label: 'nav.documents', href: '/documents', icon: FileText },
  { label: 'nav.map', href: '/map', icon: Network },
  { label: 'nav.ask', href: '/ask', icon: MessageSquareText },
];

const JOURNAL: NavPage[] = [
  { label: 'nav.journal', href: '/journal', icon: Newspaper },
  { label: 'nav.starred', href: '/journal?starred=1', icon: Star },
];

/** The pages of what is open: none on Home (no sidebar). */
export function navPages(context: OpenContext, scope: Investigation[]): NavPage[] {
  if (!context) return [];
  if (context.kind === 'journal') return JOURNAL;
  if (scope.length === 1) return scope[0].kind === 'interest' ? INTEREST : POLITICS;
  // The Map needs someone's records
  return COMPARE.filter(page => page.href !== '/map' || scope.some(i => i.kind === 'politics'));
}

/** The icons of the two kinds of investigation. */
export const KIND_ICON: Record<InvestigationKind, LucideIcon> = { politics: Landmark, interest: Hash };
