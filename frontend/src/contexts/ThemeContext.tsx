'use client';

import React, { createContext, useCallback, useContext, useEffect, useState } from 'react';

/** neon: high-contrast dark theme (deep magenta, neon green borders, purple and pink). */
export type Theme = 'light' | 'dark' | 'neon';
/** Light or dark, for what only knows those two (editor, toasts, the window's material). */
export type Appearance = 'light' | 'dark';

const STORAGE_KEY = 'app-theme';

interface ThemeContextType {
  theme: Theme;
  appearance: Appearance;
  toggleTheme: () => void;
  setTheme: (theme: Theme) => void;
}

// Default value ensures useTheme() never throws, even during SSR static page generation.
const ThemeContext = createContext<ThemeContextType>({
  theme: 'light',
  appearance: 'light',
  toggleTheme: () => {},
  setTheme: () => {},
});

const appearanceOf = (theme: Theme): Appearance => (theme === 'light' ? 'light' : 'dark');

function applyTheme(theme: Theme) {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  root.classList.toggle('dark', theme !== 'light');
  root.classList.toggle('theme-neon', theme === 'neon');
  // The window's native material (macOS vibrancy) follows the app, not the system
  import('@tauri-apps/api/window')
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(appearanceOf(theme)))
    .catch(() => {});
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setThemeState] = useState<Theme>('light');

  // Sync initial theme from localStorage on client mount
  useEffect(() => {
    let initial: Theme = 'light';
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      if (saved === 'dark' || saved === 'light' || saved === 'neon') {
        initial = saved;
      } else if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {
        initial = 'dark';
      }
    } catch {
      // Storage unavailable in restricted environment
    }
    setThemeState(initial);
    applyTheme(initial);
  }, []);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    applyTheme(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Storage unavailable
    }
  }, []);

  const toggleTheme = useCallback(() => {
    setTheme(theme === 'light' ? 'dark' : 'light');
  }, [theme, setTheme]);

  return (
    <ThemeContext.Provider value={{ theme, appearance: appearanceOf(theme), toggleTheme, setTheme }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme() {
  return useContext(ThemeContext);
}
