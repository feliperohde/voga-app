import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import type { DocumentKind, Feedback, InvestigationKind, Page } from '@/services/investigationService';

/** A document in the Feed (news, pages, videos and posts of every investigation). */
export interface FeedItem {
  id: string;
  investigationId: string;
  investigationName: string;
  url: string;
  kind: DocumentKind;
  title: string;
  site: string | null;
  publishedAt: string | null;
  fetchedAt: string;
  /** The start of the text (Markdown). */
  excerpt: string;
  /** How close it is to its investigation's interest (cosine, -1..1); null until scored. */
  relevance: number | null;
  read: boolean;
  starred: boolean;
  feedback: Feedback;
  /** The picture's address, when Settings › Collection lets it load from its site. */
  imageUrl: string | null;
  /** The copy kept on this computer. */
  imagePath: string | null;
}

export interface FeedQuery {
  /** null: every investigation. */
  investigationIds: string[] | null;
  /** Only investigations of this kind (the Journal: interests). */
  investigationKind?: InvestigationKind | null;
  sort: 'date' | 'relevance';
  /** Only what was published from then on (ISO). */
  since?: string | null;
  unreadOnly?: boolean;
  starredOnly?: boolean;
  limit?: number;
  offset?: number;
}

export const feedService = {
  list: (query: FeedQuery) =>
    invoke<Page<FeedItem>>('list_feed', { query: { investigationKind: null, since: null, unreadOnly: false, starredOnly: false, limit: null, offset: null, ...query } }),
  markRead: (ids: string[]) => invoke<void>('mark_read', { ids }),
  markAllRead: (investigationIds: string[] | null, investigationKind: InvestigationKind | null = null) => invoke<void>('mark_all_read', { investigationIds, investigationKind }),
  /** Stars a document, or says more (1) or less (-1) of this: the investigation's relevance learns from it. */
  setFlags: (id: string, flags: { starred?: boolean; feedback?: Feedback }) =>
    invoke<void>('set_document_flags', { id, starred: flags.starred ?? null, feedback: flags.feedback ?? null }),
};

/** What an item's picture is shown from: the copy kept here, else its site (as Settings › Collection allows). */
export function imageSource(item: Pick<FeedItem, 'imageUrl' | 'imagePath'>): string | null {
  if (item.imagePath) {
    try {
      return convertFileSrc(item.imagePath);
    } catch {
      // Outside the desktop app (demo in a browser)
    }
  }
  return item.imageUrl;
}
