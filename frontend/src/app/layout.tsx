'use client';

import '@fontsource-variable/source-sans-3';
import './globals.css';
import 'sonner/dist/styles.css';
import { Toaster } from 'sonner';
import { Sidebar } from '@/components/Sidebar';
import { MainContent } from '@/components/MainContent';
import { TooltipProvider } from '@/components/ui/tooltip';
import { ConfirmProvider } from '@/components/ui/confirm-dialog';
import { UpdateProvider } from '@/components/UpdateProvider';
import { ThemeProvider, useTheme } from '@/contexts/ThemeContext';
import { InvestigationProvider } from '@/contexts/InvestigationContext';
import { I18nProvider } from '@/i18n';

/**
 * Runs before first paint: applies the saved (or system) theme so dark mode doesn't
 * flash light, and marks the macOS window, which is translucent with no title bar
 * (tauri.macos.conf.json), so the page draws as a panel over the native material,
 * opaque if the user turned transparency off (Settings › Appearance).
 */
const THEME_BOOTSTRAP = `try{var d=document.documentElement,t=localStorage.getItem('app-theme');if(t==='neon')d.classList.add('dark','theme-neon');else if(t==='dark'||(t!=='light'&&matchMedia('(prefers-color-scheme: dark)').matches))d.classList.add('dark');if(window.__TAURI_INTERNALS__&&/Mac/.test(navigator.platform))d.classList.add('vibrancy');if(localStorage.getItem('app-transparency')==='off')d.classList.add('opaque')}catch(e){}`;

function Toasts() {
  const { appearance } = useTheme();
  return <Toaster position="bottom-right" theme={appearance} richColors closeButton />;
}

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="pt-BR" suppressHydrationWarning>
      <head>
        <title>Voga</title>
        <script dangerouslySetInnerHTML={{ __html: THEME_BOOTSTRAP }} />
      </head>
      <body className="font-sans antialiased">
        <ThemeProvider>
          <I18nProvider>
            <TooltipProvider>
              <ConfirmProvider>
                <UpdateProvider>
                  <InvestigationProvider>
                    <div className="flex">
                      <Sidebar />
                      <MainContent>{children}</MainContent>
                    </div>
                  </InvestigationProvider>
                  <Toasts />
                </UpdateProvider>
              </ConfirmProvider>
            </TooltipProvider>
          </I18nProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
