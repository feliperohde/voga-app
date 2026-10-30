'use client';

import React, { useEffect } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { SIDEBAR_WIDTH, useSidebarShown } from '@/components/Sidebar';

/** Height of the band that drags the window, as tall as the TitleBar (h-11). */
const TITLE_BAR_HEIGHT = 44;
/** What keeps its own clicks inside that band: controls and selectable content. */
const NOT_DRAGGABLE =
  'a, button, input, select, textarea, label, summary, [role], [contenteditable], [tabindex]:not([tabindex="-1"]), .select-text, .prose';

export function MainContent({ children }: { children: React.ReactNode }) {
  const sidebar = useSidebarShown();
  // macOS has no title bar: the top of the window works as one, an invisible
  // band over the page that drags the window (double-click zooms it)
  useEffect(() => {
    if (!document.documentElement.classList.contains('vibrancy')) return;
    const onMouseDown = (event: MouseEvent) => {
      const target = event.target as Element;
      if (event.button !== 0 || event.clientY > TITLE_BAR_HEIGHT) return;
      if (target.hasAttribute('data-tauri-drag-region') || target.closest(NOT_DRAGGABLE)) return;
      event.preventDefault();
      const appWindow = getCurrentWindow();
      (event.detail === 2 ? appWindow.toggleMaximize() : appWindow.startDragging()).catch(error =>
        console.error('Failed to move the window:', error),
      );
    };
    document.addEventListener('mousedown', onMouseDown);
    return () => document.removeEventListener('mousedown', onMouseDown);
  }, []);

  return (
    // The page fills the window and is opaque, even where the window lets the
    // material through (macOS); only the sidebar shows it
    <main className="min-w-0 flex-1 overflow-hidden" style={{ marginLeft: sidebar ? SIDEBAR_WIDTH : 0 }}>
      <div className="h-screen w-full min-w-0 max-w-full overflow-y-auto overflow-x-hidden bg-background">{children}</div>
    </main>
  );
}
